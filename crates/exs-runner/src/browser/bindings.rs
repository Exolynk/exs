//! JavaScript imports used by the browser execution backend.

use super::*;

#[wasm_bindgen(inline_js = r#"
const HOST_CALL_READY = 0;
const HOST_CALL_PENDING = 1;
const HOST_CALL_UNSUPPORTED = 3;
const PAGE_SIZE = 65536;
const RESULT_VALUE_EXPORT = "__exs_result_value";
const STATUS_COMPLETE = 2;
const STATUS_PENDING = 1;
const STATUS_CANCELLED = 3;

function bytes(value, label) {
    if (value instanceof Uint8Array) {
        return value;
    }
    throw new TypeError(`${label} must be a Uint8Array`);
}

function range(memory, pointer, length, label) {
    if (!Number.isInteger(pointer) || !Number.isInteger(length) || pointer < 0 || length < 0) {
        throw new RangeError(`${label} has an invalid memory range`);
    }
    const end = pointer + length;
    if (!Number.isSafeInteger(end) || end > memory.buffer.byteLength) {
        throw new RangeError(`${label} lies outside linear memory`);
    }
    return [pointer, end];
}

function read(memory, pointer, length, label) {
    const [start, end] = range(memory, pointer, length, label);
    return new Uint8Array(memory.buffer.slice(start, end));
}

function write(memory, pointer, value, label) {
    const output = bytes(value, label);
    const [start] = range(memory, pointer, output.length, label);
    new Uint8Array(memory.buffer).set(output, start);
}

function valueReader(memory, value, vm) {
    const bits = BigInt.asUintN(64, BigInt(value));
    return {
        low: Number(bits & 0xffffffffn),
        high: Number(bits >> 32n),
        vm: vm >>> 0,
        size: memory.buffer.byteLength,
        page(index) {
            const start = index * PAGE_SIZE;
            const end = Math.min(start + PAGE_SIZE, memory.buffer.byteLength);
            if (!Number.isInteger(index) || index < 0 || start >= end) {
                throw new RangeError("memory page lies outside linear memory");
            }
            return new Uint8Array(memory.buffer.slice(start, end));
        },
    };
}

function exportedFunction(exports, name) {
    const value = exports[name];
    if (typeof value !== "function") {
        throw new TypeError(`missing or invalid ExS ABI export ${name}`);
    }
    return value;
}

export async function createBrowserRunner(wasm, host, release, expectedAbiVersion) {
    const module = await WebAssembly.compile(bytes(wasm, "compiled ExS module"));
    if (typeof host !== "function") {
        throw new TypeError("browser host dispatcher must be a function");
    }
    if (typeof release !== "function") {
        throw new TypeError("browser host cleanup callback must be a function");
    }
    const direct = WebAssembly.Module.exports(module).some((entry) => entry.name === RESULT_VALUE_EXPORT);
    let nextExecutionId = 1;
    return {
        direct,
        async execute(functionName, input) {
            if (nextExecutionId > 2_147_483_647) {
                throw new RangeError("browser execution identity overflow");
            }
            const executionId = nextExecutionId++;
            const executionStartedAt = performance.now();
            const ready = new Map();
            const pending = new Map();
            let activeTasks = 0;
            let memory;
            const hostName = (pointer, length) => new TextDecoder("utf-8", { fatal: true }).decode(
                read(memory, pointer, length, "host function name"),
            );
            const startHostCall = (callId, name, request, sourcePosition) => {
                const response = host(name, request, sourcePosition, executionId, executionStartedAt);
                if (response === null) {
                    return HOST_CALL_UNSUPPORTED;
                }
                if (response && typeof response.then === "function") {
                    pending.set(
                        callId,
                        Promise.resolve(response).then((value) => ({ callId, value: bytes(value, "host response") })),
                    );
                    return HOST_CALL_PENDING;
                }
                ready.set(callId, bytes(response, "host response"));
                return HOST_CALL_READY;
            };
            try {
                const imports = {
                exs: {
                    __exs_host_call_start(callId, namePointer, nameLength, requestPointer, requestLength, sourcePosition) {
                        const request = read(memory, requestPointer, requestLength, "host-call request");
                        return startHostCall(callId, hostName(namePointer, nameLength), request, sourcePosition);
                    },
                    __exs_host_call_start_value(callId, namePointer, nameLength, args, vm, sourcePosition) {
                        const request = valueReader(memory, args, vm);
                        return startHostCall(callId, hostName(namePointer, nameLength), request, sourcePosition);
                    },
                    __exs_host_call_response_len(callId) {
                        const response = ready.get(callId);
                        if (!response) {
                            throw new Error("host response is not ready");
                        }
                        return response.length;
                    },
                    __exs_host_call_response_copy(callId, pointer, length) {
                        const response = ready.get(callId);
                        if (!response || response.length !== length) {
                            throw new Error("host response has an unexpected length");
                        }
                        write(memory, pointer, response, "host response destination");
                        ready.delete(callId);
                        return 0;
                    },
                },
                runner: {
                    __runner_task_acquire() {
                        if (activeTasks >= Number.MAX_SAFE_INTEGER) {
                            throw new RangeError("browser task counter overflow");
                        }
                        activeTasks += 1;
                        return 0;
                    },
                    __runner_task_release() {
                        if (activeTasks === 0) {
                            return 1;
                        }
                        activeTasks -= 1;
                        return 0;
                    },
                },
                };
                const instance = await WebAssembly.instantiate(module, imports);
                memory = instance.exports.memory;
                if (!(memory instanceof WebAssembly.Memory)) {
                throw new TypeError("missing exported ExS linear memory");
                }
                const version = exportedFunction(instance.exports, "__exs_abi_version")();
                if (version !== expectedAbiVersion) {
                throw new TypeError(`expected ExS ABI version ${expectedAbiVersion}, received ${version}`);
                }
                const inputBytes = bytes(input, "ExS input");
                const allocate = exportedFunction(instance.exports, "__exs_input_alloc");
                const inputPointer = allocate(inputBytes.length);
                write(memory, inputPointer, inputBytes, "ExS input");
                if (typeof functionName !== "string" || functionName.length === 0) {
                throw new TypeError("ExS function name must be a non-empty string");
                }
                const start = exportedFunction(instance.exports, `__exs_start_${functionName}`);
                const resultPointer = () => exportedFunction(instance.exports, "__exs_result_ptr")();
                const resultLength = () => exportedFunction(instance.exports, "__exs_result_len")();
                let status = start(inputPointer, inputBytes.length);
                while (true) {
                if (status === STATUS_COMPLETE) {
                    if (direct) {
                        const vm = exportedFunction(instance.exports, "__exs_vm")();
                        return valueReader(memory, exportedFunction(instance.exports, RESULT_VALUE_EXPORT)(), vm);
                    }
                    return read(memory, resultPointer(), resultLength(), "ExS result");
                }
                if (status === STATUS_CANCELLED) {
                    throw new Error("ExS execution was cancelled");
                }
                if (status !== STATUS_PENDING) {
                    throw new Error(`unexpected ExS execution status ${status}`);
                }
                if (pending.size === 0) {
                    throw new Error("ExS execution is pending without a host Promise");
                }
                const { callId, value } = await Promise.race(pending.values());
                pending.delete(callId);
                const responsePointer = allocate(value.length);
                write(memory, responsePointer, value, "host response");
                const resume = exportedFunction(instance.exports, "__exs_resume_host");
                status = resume(callId, responsePointer, value.length);
                }
            } finally {
                release(executionId);
            }
        },
    };
}

export function browserRunnerDirect(controller) {
    return Boolean(controller && controller.direct);
}

export function executeBrowserRunner(controller, functionName, input) {
    if (!controller || typeof controller.execute !== "function") {
        throw new TypeError("invalid ExS browser runner controller");
    }
    return controller.execute(functionName, input);
}
"#)]
extern "C" {
    /// Creates one JavaScript controller around a browser-compiled ExS module.
    #[wasm_bindgen(catch, js_name = createBrowserRunner)]
    pub(super) fn create_browser_runner(
        wasm: &Uint8Array,
        host: &Function,
        release: &Function,
        expected_abi_version: i32,
    ) -> Result<Promise, JsValue>;

    /// Returns whether the controller's module uses the direct boundary of runtime v2.
    #[wasm_bindgen(js_name = browserRunnerDirect)]
    pub(super) fn browser_runner_direct(controller: &JsValue) -> bool;

    /// Executes one isolated ExS instance through the JavaScript controller.
    #[wasm_bindgen(catch, js_name = executeBrowserRunner)]
    pub(super) fn execute_browser_runner(
        controller: &JsValue,
        function: &str,
        input: &Uint8Array,
    ) -> Result<Promise, JsValue>;
}

#[wasm_bindgen]
extern "C" {
    /// A value in the memory of a runtime v2 module, whose pages JavaScript copies on demand.
    pub(super) type ValueReader;

    /// Returns the low 32 bits of the value.
    #[wasm_bindgen(method, getter)]
    pub(super) fn low(this: &ValueReader) -> u32;

    /// Returns the high 32 bits of the value.
    #[wasm_bindgen(method, getter)]
    pub(super) fn high(this: &ValueReader) -> u32;

    /// Returns the address of the module's VM state block.
    #[wasm_bindgen(method, getter)]
    pub(super) fn vm(this: &ValueReader) -> u32;

    /// Returns the byte size of the module's memory.
    #[wasm_bindgen(method, getter)]
    pub(super) fn size(this: &ValueReader) -> f64;

    /// Returns a copy of 64 KiB page `index` of the module's memory.
    #[wasm_bindgen(method, catch)]
    pub(super) fn page(this: &ValueReader, index: u32) -> Result<Uint8Array, JsValue>;
}

#[wasm_bindgen(inline_js = r#"
export function exsHostNow() {
    const now = new Date();
    const milliseconds = now.getTime();
    let timezone = null;
    try {
        const candidate = new Intl.DateTimeFormat().resolvedOptions().timeZone;
        if (typeof candidate === "string" && candidate.length > 0) {
            timezone = candidate;
        }
    } catch (_) {
        // An IANA identifier is optional, but the instant and offset are not.
    }
    return [
        Math.floor(milliseconds / 1_000),
        now.getMilliseconds() * 1_000_000,
        -now.getTimezoneOffset() * 60,
        timezone,
    ];
}

export function exsHostElapsed(startedAt) {
    const elapsedMilliseconds = Math.max(0, performance.now() - startedAt);
    const seconds = Math.floor(elapsedMilliseconds / 1_000);
    const nanoseconds = Math.floor((elapsedMilliseconds - seconds * 1_000) * 1_000_000);
    return [seconds, nanoseconds];
}

export function exsHostSleep(seconds, nanoseconds) {
    const maxMilliseconds = 2_147_483_647n;
    let remainingNanoseconds = BigInt(seconds) * 1_000_000_000n + BigInt(nanoseconds);
    return new Promise((resolve) => {
        const schedule = () => {
            if (remainingNanoseconds === 0n) {
                resolve();
                return;
            }
            const milliseconds = remainingNanoseconds / 1_000_000n;
            const delay = milliseconds > maxMilliseconds ? maxMilliseconds : milliseconds;
            const delayNanoseconds = delay * 1_000_000n;
            if (delayNanoseconds === 0n) {
                remainingNanoseconds = 0n;
                globalThis.setTimeout(schedule, 0);
                return;
            }
            remainingNanoseconds -= delayNanoseconds;
            globalThis.setTimeout(schedule, Number(delay));
        };
        schedule();
    });
}
"#)]
extern "C" {
    /// Returns one browser wall-clock snapshot from Date and Intl.
    #[wasm_bindgen(js_name = exsHostNow)]
    pub(super) fn browser_host_now() -> JsValue;

    /// Returns monotonic elapsed Duration parts from performance.now.
    #[wasm_bindgen(js_name = exsHostElapsed)]
    pub(super) fn browser_host_elapsed(execution_started_at: f64) -> JsValue;

    /// Starts one browser-native timeout Promise for validated Duration parts.
    #[wasm_bindgen(catch, js_name = exsHostSleep)]
    pub(super) fn browser_host_sleep(seconds: u64, nanoseconds: u32) -> Result<Promise, JsValue>;
}
