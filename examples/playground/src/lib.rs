//! A browser-only ExS source editor and execution example for Trunk.

#[cfg(target_arch = "wasm32")]
mod browser {
    use exs_compiler::{CompileOptions, SourceInput, compile};
    use exs_runner::{BrowserRunner, BrowserRunnerConfig, ExsValue};
    use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
    use wasm_bindgen_futures::spawn_local;
    use web_sys::{
        Document, Event, HtmlButtonElement, HtmlElement, HtmlTextAreaElement, KeyboardEvent, Window,
    };

    /// Starts the interactive browser playground after the Trunk bundle loads.
    ///
    /// # Errors
    ///
    /// Returns a JavaScript error when the page does not contain the required controls.
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        let document = browser_document()?;
        let source = element_by_id::<HtmlTextAreaElement>(&document, "source")?;
        let output = element_by_id::<HtmlElement>(&document, "output")?;
        let button = element_by_id::<HtmlButtonElement>(&document, "run")?;

        let callback_source = source.clone();
        let callback_output = output.clone();
        let callback_button = button.clone();
        let callback = Closure::wrap(Box::new(move |_event: Event| {
            callback_button.set_disabled(true);
            callback_output.set_text_content(Some("Compiling...\n"));
            let source = callback_source.value();
            let output = callback_output.clone();
            let button = callback_button.clone();
            spawn_local(async move {
                execute_source(source, output).await;
                button.set_disabled(false);
            });
        }) as Box<dyn FnMut(Event)>);
        button.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
        callback.forget();

        let tab_source = source.clone();
        let tab_callback = Closure::wrap(Box::new(move |event: KeyboardEvent| {
            if event.key() == "Tab" && !event.shift_key() {
                event.prevent_default();
                let _ignored_error = insert_indent(&tab_source);
            }
        }) as Box<dyn FnMut(KeyboardEvent)>);
        source
            .add_event_listener_with_callback("keydown", tab_callback.as_ref().unchecked_ref())?;
        tab_callback.forget();
        Ok(())
    }

    /// Returns the document for the browser window hosting this playground.
    fn browser_document() -> Result<Document, JsValue> {
        let window: Window =
            web_sys::window().ok_or_else(|| JsValue::from_str("window is unavailable"))?;
        window
            .document()
            .ok_or_else(|| JsValue::from_str("document is unavailable"))
    }

    /// Returns one required DOM element after checking its expected concrete type.
    fn element_by_id<ElementType>(document: &Document, id: &str) -> Result<ElementType, JsValue>
    where
        ElementType: JsCast,
    {
        let element = document
            .get_element_by_id(id)
            .ok_or_else(|| JsValue::from_str(&format!("missing #{id} control")))?;
        element
            .dyn_into::<ElementType>()
            .map_err(|_| JsValue::from_str(&format!("#{id} has an unexpected element type")))
    }

    /// Replaces the active editor selection with one four-space indentation and restores its caret.
    fn insert_indent(source: &HtmlTextAreaElement) -> Result<(), JsValue> {
        let start = source
            .selection_start()?
            .ok_or_else(|| JsValue::from_str("source selection start is unavailable"))?;
        let end = source
            .selection_end()?
            .ok_or_else(|| JsValue::from_str("source selection end is unavailable"))?;
        source.set_range_text_with_start_and_end("    ", start, end)?;
        let caret = start
            .checked_add(4)
            .ok_or_else(|| JsValue::from_str("source selection is too large"))?;
        source.set_selection_range(caret, caret)
    }

    /// Compiles and executes the source supplied through the playground editor.
    async fn execute_source(source: String, output: HtmlElement) {
        let compiled = match compile(
            SourceInput {
                source_id: "playground.exs",
                text: &source,
            },
            CompileOptions::default(),
        ) {
            Ok(compiled) => compiled,
            Err(diagnostics) => {
                output.set_text_content(Some(&format!("Compilation failed:\n{diagnostics}")));
                return;
            }
        };

        let mut configuration = BrowserRunnerConfig::new();
        if let Err(error) = register_output_hosts(&mut configuration, &output) {
            output.set_text_content(Some(&format!("Could not configure output: {error}")));
            return;
        }
        let runner = match BrowserRunner::new(&compiled.wasm, configuration).await {
            Ok(runner) => runner,
            Err(error) => {
                output.set_text_content(Some(&format!("Could not prepare execution: {error}")));
                return;
            }
        };
        match runner.execute(&[]).await {
            Ok(result) => append_output(&output, &format!("Result: {result:?}\n")),
            Err(error) => append_output(&output, &format!("Execution failed: {error}\n")),
        }
    }

    /// Registers the `print` and `println` host functions used by playground programs.
    fn register_output_hosts(
        configuration: &mut BrowserRunnerConfig,
        output: &HtmlElement,
    ) -> Result<(), exs_runner::BrowserRegistryError> {
        let print_output = output.clone();
        configuration
            .registry_mut()
            .register_sync("print", move |arguments| {
                append_output(&print_output, &format_arguments(&arguments));
                ExsValue::None
            })?;

        let println_output = output.clone();
        configuration
            .registry_mut()
            .register_sync("println", move |arguments| {
                append_output(
                    &println_output,
                    &format!("{}\n", format_arguments(&arguments)),
                );
                ExsValue::None
            })
    }

    /// Appends one message to the browser output panel without replacing prior host output.
    fn append_output(output: &HtmlElement, message: &str) {
        let mut existing = output.text_content().unwrap_or_default();
        existing.push_str(message);
        output.set_text_content(Some(&existing));
    }

    /// Formats one host-call argument list for readable browser-console output.
    fn format_arguments(arguments: &[ExsValue]) -> String {
        arguments
            .iter()
            .map(format_value)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Formats one ExS host value with unquoted strings and debug output for compound values.
    fn format_value(value: &ExsValue) -> String {
        match value {
            ExsValue::String(value) => value.clone(),
            value => format!("{value:?}"),
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::start;
