//! A browser-only ExS source editor, documentation browser, and execution example for Trunk.

#[cfg(any(target_arch = "wasm32", test))]
mod documentation;
#[cfg(any(target_arch = "wasm32", test))]
mod markdown;

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::sync::Arc;

    use birei::CodeEditor;
    use birei::code_editor::CodeLanguageService;
    use exs_autocomplete::ExsBireiLanguageService;
    use exs_compiler::{CompileOptions, DocumentationPage, SourceInput, compile, format};
    use exs_runner::{BrowserRunner, BrowserRunnerConfig, Bytes, ExsError, ExsValue};
    use leptos::prelude::*;
    use leptos::task::spawn_local;
    use serde::{Deserialize, Serialize};
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    use web_sys::Element;

    use crate::documentation::{PLAYGROUND_SOURCE_ID, documentation_pages};
    use crate::markdown::render_documentation_markdown;

    const DEFAULT_SOURCE: &str = r#"type Vehicle {
    id: String,
    registration: String,
    status: VehicleStatus,
    odometer_km: Int,
    maintenance_dates: List<String>,
    inspection_photo: Bytes,
    note: String | None,
}

enum VehicleStatus {
    Available,
    InService(until: String),
    Retired(reason: String),
}

type ServiceVisit {
    id: String,
    performed_on: String,
    operations: List<String>,
    invoice_pdf: Bytes,
    advisor: String | None,
}

fn main() -> Vehicle | Error {
    let vehicle: Vehicle = Host::call("fleet.vehicle", "veh_0042")?;
    Host::call("println", "Vehicle", vehicle.registration, "at", vehicle.odometer_km, "km");

    let visits = Host::stream("fleet.visits", vehicle.id)?;
    for visit: ServiceVisit in visits {
        Host::call("println", visit.performed_on, visit.operations.length(), "operations");
    }

    ret vehicle;
}"#;

    /// One host-side vehicle DTO transferred through the Serde wire adapter.
    #[derive(Clone, Deserialize, Serialize)]
    struct Vehicle {
        /// Stable fleet identifier.
        id: String,
        /// Human-readable license registration.
        registration: String,
        /// Current operational status.
        status: VehicleStatus,
        /// Most recently recorded odometer value.
        odometer_km: i64,
        /// ISO-8601 dates of recent maintenance activity.
        maintenance_dates: Vec<String>,
        /// Binary inspection photo attachment.
        inspection_photo: Bytes,
        /// Optional service desk note.
        note: Option<String>,
    }

    /// One vehicle lifecycle state transferred as a tagged ExS Object.
    #[derive(Clone, Deserialize, Serialize)]
    enum VehicleStatus {
        /// Vehicle may be scheduled immediately.
        Available,
        /// Vehicle is reserved for maintenance through an ISO-8601 date.
        InService {
            /// Planned completion date.
            until: String,
        },
        /// Vehicle was permanently removed from active service.
        Retired {
            /// Human-readable retirement reason.
            reason: String,
        },
    }

    /// One service visit transferred through the host iterator boundary.
    #[derive(Clone, Deserialize, Serialize)]
    struct ServiceVisit {
        /// Stable visit identifier.
        id: String,
        /// ISO-8601 completion date.
        performed_on: String,
        /// Completed work entries.
        operations: Vec<String>,
        /// Binary invoice attachment.
        invoice_pdf: Bytes,
        /// Optional named service advisor.
        advisor: Option<String>,
    }

    /// Entry page for the generated standard-library documentation.
    const STANDARD_DOCUMENTATION_INDEX: &str = "modules/std/index.md";

    /// Selects the currently visible auxiliary playground panel.
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum SidePanel {
        /// Displays stdout, results, and execution diagnostics.
        Output,
        /// Displays generated built-in and current-source documentation.
        Documentation,
    }

    /// Starts the interactive browser playground after the Trunk bundle loads.
    ///
    /// # Errors
    ///
    /// Returns a JavaScript error when Birei's embedded assets cannot be added to the page.
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        birei::embed_assets()?;
        mount_to_body(Playground);
        Ok(())
    }

    /// Renders the reactive editor, execution output, and generated documentation browser.
    #[component]
    fn Playground() -> impl IntoView {
        let source = RwSignal::new(DEFAULT_SOURCE.to_owned());
        let (initial_pages, initial_error) = match documentation_pages(DEFAULT_SOURCE) {
            Ok(pages) => (pages, String::new()),
            Err(error) => (Vec::new(), error),
        };
        let documentation = RwSignal::new(initial_pages);
        let selected_documentation = RwSignal::new(String::from(STANDARD_DOCUMENTATION_INDEX));
        let documentation_error = RwSignal::new(initial_error);
        let editor_status = RwSignal::new(String::new());
        let output = RwSignal::new(String::from("Ready.\n"));
        let running = RwSignal::new(false);
        let active_panel = RwSignal::new(SidePanel::Documentation);
        let language_service: Arc<dyn CodeLanguageService> =
            Arc::new(ExsBireiLanguageService::default());

        let refresh_documentation = move |next_source: &str| {
            refresh_documentation_pages(
                next_source,
                documentation,
                selected_documentation,
                documentation_error,
            );
        };

        let source_for_change = source;
        let source_for_run = source;
        view! {
            <main class="playground-shell">
                <header class="playground-header">
                    <div>
                        <h1>"ExS Playground"</h1>
                        <p>"Write, format, and run ExS in your browser."</p>
                    </div>
                    <button
                        type="button"
                        class="run-button"
                        disabled=move || running.get()
                        on:click=move |_| {
                            active_panel.set(SidePanel::Output);
                            running.set(true);
                            output.set(String::from("Compiling...\n"));
                            let source = source_for_run.get_untracked();
                            spawn_local(async move {
                                execute_source(source, output).await;
                                running.set(false);
                            });
                        }
                    >
                        {move || if running.get() { "Running..." } else { "Run" }}
                    </button>
                </header>

                <div class="playground-layout">
                    <section class="editor-panel" aria-label="ExS source editor">
                        <div class="panel-heading">
                            <h2>"Source"</h2>
                            <span class="editor-status">{move || editor_status.get()}</span>
                        </div>
                        <div class="editor-surface">
                            <CodeEditor
                                id="source"
                                value=source
                                service=language_service
                                line_numbers=true
                                placeholder="Write ExS source..."
                                on_input=Callback::new(move |next| source_for_change.set(next))
                                on_change=Callback::new(move |next: String| {
                                    match format(SourceInput {
                                        source_id: PLAYGROUND_SOURCE_ID,
                                        text: &next,
                                    }) {
                                        Ok(formatted) => {
                                            source_for_change.set(formatted.clone());
                                            editor_status.set(String::from("Formatted"));
                                            refresh_documentation(&formatted);
                                        }
                                        Err(diagnostics) => {
                                            editor_status.set(diagnostics.render(&next));
                                        }
                                    }
                                })
                            />
                        </div>
                    </section>

                    <aside class="side-panel">
                        <div class="side-tabs" role="tablist" aria-label="Playground details">
                            <button
                                type="button"
                                class=move || side_tab_class(active_panel.get() == SidePanel::Output)
                                aria-selected=move || active_panel.get() == SidePanel::Output
                                on:click=move |_| active_panel.set(SidePanel::Output)
                            >"Output"</button>
                            <button
                                type="button"
                                class=move || side_tab_class(active_panel.get() == SidePanel::Documentation)
                                aria-selected=move || active_panel.get() == SidePanel::Documentation
                                on:click=move |_| {
                                    selected_documentation.set(String::from(STANDARD_DOCUMENTATION_INDEX));
                                    active_panel.set(SidePanel::Documentation);
                                }
                            >"Doc"</button>
                        </div>

                        <section
                            class=move || side_panel_class(
                                "output-panel",
                                active_panel.get() == SidePanel::Output,
                            )
                            aria-label="Program output"
                        >
                            <pre class="output-content" aria-live="polite">{move || output.get()}</pre>
                        </section>

                        <section
                            class=move || side_panel_class(
                                "documentation-panel",
                                active_panel.get() == SidePanel::Documentation,
                            )
                            aria-label="Generated documentation"
                        >
                            <p class="documentation-error">{move || documentation_error.get()}</p>
                            <div
                                class="documentation-content"
                                on:click=move |event| {
                                    select_documentation_link(event, selected_documentation);
                                }
                                inner_html=move || selected_documentation_html(
                                    &documentation.get(),
                                    &selected_documentation.get(),
                                )
                            ></div>
                        </section>
                    </aside>
                </div>
            </main>
        }
    }

    /// Replaces documentation pages after a successful formatting pass while retaining the selection.
    fn refresh_documentation_pages(
        source: &str,
        documentation: RwSignal<Vec<DocumentationPage>>,
        selected_documentation: RwSignal<String>,
        documentation_error: RwSignal<String>,
    ) {
        match documentation_pages(source) {
            Ok(pages) => {
                let selected = selected_documentation.get_untracked();
                if !pages.iter().any(|page| page.path == selected) {
                    let next_selection = pages
                        .first()
                        .map(|page| page.path.clone())
                        .unwrap_or_default();
                    selected_documentation.set(next_selection);
                }
                documentation.set(pages);
                documentation_error.set(String::new());
            }
            Err(error) => documentation_error.set(error),
        }
    }

    /// Renders the selected generated documentation page as safe browser HTML.
    fn selected_documentation_html(pages: &[DocumentationPage], selected: &str) -> String {
        pages
            .iter()
            .find(|page| page.path == selected)
            .map(|page| render_documentation_markdown(&page.markdown, &page.path, pages))
            .unwrap_or_else(|| String::from("<p>Documentation is unavailable.</p>"))
    }

    /// Selects a generated documentation page after an internal rendered link is clicked.
    fn select_documentation_link(event: leptos::ev::MouseEvent, selected: RwSignal<String>) {
        let link = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
            .and_then(|target| target.closest("[data-documentation-page]").ok().flatten());
        let Some(link) = link else {
            return;
        };
        let Some(page) = link.get_attribute("data-documentation-page") else {
            return;
        };
        event.prevent_default();
        selected.set(page);
    }

    /// Returns the CSS class list for one side-panel tab.
    fn side_tab_class(active: bool) -> &'static str {
        if active {
            "side-tab side-tab--active"
        } else {
            "side-tab"
        }
    }

    /// Returns the CSS class list that hides inactive side panels without removing their state.
    fn side_panel_class(panel: &str, active: bool) -> String {
        if active {
            format!("{panel} side-panel-content side-panel-content--active")
        } else {
            format!("{panel} side-panel-content")
        }
    }

    /// Compiles and executes source supplied through the playground editor.
    async fn execute_source(source: String, output: RwSignal<String>) {
        let compiled = match compile(
            SourceInput {
                source_id: PLAYGROUND_SOURCE_ID,
                text: &source,
            },
            CompileOptions::default(),
        ) {
            Ok(compiled) => compiled,
            Err(diagnostics) => {
                output.set(format!(
                    "Compilation failed:\n{}",
                    diagnostics.render(&source)
                ));
                return;
            }
        };

        let mut configuration = BrowserRunnerConfig::new();
        if let Err(error) = register_output_hosts(&mut configuration, output) {
            output.set(format!("Could not configure output: {error}"));
            return;
        }
        if let Err(error) = register_fleet_hosts(&mut configuration) {
            output.set(format!("Could not configure fleet services: {error}"));
            return;
        }
        let runner = match BrowserRunner::new(&compiled.wasm, configuration).await {
            Ok(runner) => runner,
            Err(error) => {
                output.set(format!("Could not prepare execution: {error}"));
                return;
            }
        };
        match runner.execute("main", &[]).await {
            Ok(result) => append_output(output, &format!("Result: {result:?}\n")),
            Err(error) => append_output(output, &format!("Execution failed: {error}\n")),
        }
    }

    /// Registers the `print` and `println` host functions used by playground programs.
    fn register_output_hosts(
        configuration: &mut BrowserRunnerConfig,
        output: RwSignal<String>,
    ) -> Result<(), exs_runner::BrowserRegistryError> {
        configuration
            .registry_mut()
            .fn_sync_raw("print", move |arguments| {
                append_output(output, &format_arguments(&arguments));
                ExsValue::None
            })?;

        configuration
            .registry_mut()
            .fn_sync_raw("println", move |arguments| {
                append_output(output, &format!("{}\n", format_arguments(&arguments)));
                ExsValue::None
            })
    }

    /// Registers typed fleet DTO services used by the default playground program.
    fn register_fleet_hosts(
        configuration: &mut BrowserRunnerConfig,
    ) -> Result<(), exs_runner::BrowserRegistryError> {
        configuration.registry_mut().fn_sync(
            "fleet.vehicle",
            |id: String| -> Result<Vehicle, ExsError> {
                Ok(Vehicle {
                    id,
                    registration: "ZH 421 742".into(),
                    status: VehicleStatus::InService {
                        until: "2026-09-04".into(),
                    },
                    odometer_km: 48_120,
                    maintenance_dates: vec!["2026-02-12".into(), "2026-08-20".into()],
                    inspection_photo: Bytes::new(vec![0x89, 0x50, 0x4e, 0x47]),
                    note: Some("Awaiting replacement front sensor.".into()),
                })
            },
        )?;
        configuration.registry_mut().stream(
            "fleet.visits",
            |_vehicle_id: String| -> Result<Vec<ServiceVisit>, ExsError> {
                Ok(vec![
                    ServiceVisit {
                        id: "svc_2026_08_20".into(),
                        performed_on: "2026-08-20".into(),
                        operations: vec!["replace brake pads".into(), "calibrate radar".into()],
                        invoice_pdf: Bytes::new(vec![0x25, 0x50, 0x44, 0x46]),
                        advisor: Some("M. Keller".into()),
                    },
                    ServiceVisit {
                        id: "svc_2026_02_12".into(),
                        performed_on: "2026-02-12".into(),
                        operations: vec!["annual inspection".into()],
                        invoice_pdf: Bytes::new(vec![0x25, 0x50, 0x44, 0x46, 0x2d, 0x32]),
                        advisor: None,
                    },
                ])
            },
        )
    }

    /// Appends one message to the browser output panel without replacing prior host output.
    fn append_output(output: RwSignal<String>, message: &str) {
        output.update(|existing| existing.push_str(message));
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
