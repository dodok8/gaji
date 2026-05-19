use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};
use rquickjs::loader::{Loader, Resolver};
use rquickjs::{function::Func, Context as JsContext, Module, Runtime as JsRuntime};

#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub id: String,
    pub json: String,
    /// "workflow" or "action"
    pub output_type: String,
}

pub fn strip_typescript(source: &str, filename: &str) -> Result<String> {
    let allocator = Allocator::default();
    let source_type =
        SourceType::from_path(Path::new(filename)).unwrap_or_else(|_| SourceType::tsx());

    let parser_ret = Parser::new(&allocator, source, source_type).parse();
    if !parser_ret.errors.is_empty() {
        let errors: Vec<String> = parser_ret.errors.iter().map(|e| e.to_string()).collect();
        return Err(anyhow::anyhow!("Parse errors:\n{}", errors.join("\n")));
    }

    let mut program = parser_ret.program;

    let semantic_ret = SemanticBuilder::new().build(&program);
    let scoping = semantic_ret.semantic.into_scoping();

    let transform_options = TransformOptions::default();
    let _transformer_ret = Transformer::new(&allocator, Path::new(filename), &transform_options)
        .build_with_scoping(scoping, &mut program);

    let code = Codegen::new().build(&program).code;
    Ok(code)
}

/// The source cache is shared across multiple `execute_workflow` and
/// `execute_config` calls, so TypeScript stripping of a common module (e.g.
/// `lib/common.ts`) is performed only once per process run.
#[derive(Clone, Default)]
pub struct ModuleResolver {
    cache: Arc<Mutex<HashMap<PathBuf, String>>>,
}

impl Resolver for ModuleResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &rquickjs::Ctx<'js>,
        base: &str,
        name: &str,
    ) -> rquickjs::Result<String> {
        let joined = Path::new(base).parent().unwrap_or(Path::new(".")).join(name);
        if let Ok(p) = joined.canonicalize() {
            return Ok(p.to_string_lossy().into_owned());
        }
        // .js → .ts fallback: TypeScript ESM convention writes imports as .js.
        if joined.extension().is_some_and(|e| e == "js") {
            if let Ok(p) = joined.with_extension("ts").canonicalize() {
                return Ok(p.to_string_lossy().into_owned());
            }
        }
        // Stub out generated/index.js when it doesn't exist yet (fresh project).
        if joined.ends_with("generated/index.js") {
            return Ok("__gaji_generated_stub__".to_string());
        }
        Err(rquickjs::Error::new_resolving(base, name))
    }
}

impl Loader for ModuleResolver {
    fn load<'js>(
        &mut self,
        ctx: &rquickjs::Ctx<'js>,
        name: &str,
    ) -> rquickjs::Result<Module<'js>> {
        if name == "__gaji_generated_stub__" {
            return Module::declare(
                ctx.clone(),
                name,
                "export const defineConfig = function(c) { return c; };",
            );
        }

        let path = PathBuf::from(name);

        let source = {
            let mut cache = self.cache.lock().unwrap();
            if let Some(cached) = cache.get(&path) {
                cached.clone()
            } else {
                let raw = std::fs::read_to_string(&path)
                    .map_err(|e| rquickjs::Error::new_loading_message(name, e.to_string()))?;
                let stripped = if path.extension().is_some_and(|e| e == "ts") {
                    let filename = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    strip_typescript(&raw, &filename)
                        .map_err(|e| rquickjs::Error::new_loading_message(name, e.to_string()))?
                } else {
                    raw
                };
                cache.insert(path.clone(), stripped.clone());
                stripped
            }
        };

        Module::declare(ctx.clone(), name, source.as_str())
    }
}


/// The `resolver` holds a shared source cache so TypeScript stripping of common
/// imported modules is done only once across multiple workflow builds in a single
/// run. Any unresolvable import causes an error, which the caller should treat
/// as a signal to fall back to Node.js.
pub fn execute_workflow(
    resolver: &mut ModuleResolver,
    workflow_path: &Path,
) -> Result<Vec<BuildOutput>> {
    let canonical = workflow_path
        .canonicalize()
        .context("Failed to canonicalize workflow path")?;

    let raw = std::fs::read_to_string(&canonical)
        .with_context(|| format!("Failed to read workflow: {}", canonical.display()))?;
    let source = if canonical.extension().is_some_and(|e| e == "ts") {
        let filename = canonical
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        strip_typescript(&raw, &filename)
            .with_context(|| format!("Failed to strip TypeScript: {}", canonical.display()))?
    } else {
        raw
    };

    let outputs: Rc<RefCell<Vec<BuildOutput>>> = Rc::new(RefCell::new(Vec::new()));

    {
        let loader = resolver.clone();
        let rt = JsRuntime::new().context("Failed to create QuickJS runtime")?;
        rt.set_loader(loader.clone(), loader);
        let ctx = JsContext::full(&rt).context("Failed to create QuickJS context")?;

        ctx.with(|ctx| {
            let outputs_clone = outputs.clone();
            let build_fn = Func::from(
                move |id: String, json: String, output_type: rquickjs::function::Opt<String>| {
                    outputs_clone.borrow_mut().push(BuildOutput {
                        id,
                        json,
                        output_type: output_type.0.unwrap_or_else(|| "workflow".to_string()),
                    });
                },
            );
            ctx.globals()
                .set("__gha_build", build_fn)
                .map_err(|e| anyhow::anyhow!("Failed to set __gha_build: {}", e))?;

            let entry_name = canonical.to_string_lossy().into_owned();
            Module::evaluate(ctx.clone(), entry_name, source.as_str())
                .map_err(|e| anyhow::anyhow!("QuickJS workflow module error: {}", e))?
                .finish::<()>()
                .map_err(|e| anyhow::anyhow!("QuickJS workflow evaluation error: {}", e))?;

            Ok::<_, anyhow::Error>(())
        })?;
    }

    let result = Rc::try_unwrap(outputs)
        .map_err(|_| anyhow::anyhow!("Failed to unwrap Rc - references still held"))?
        .into_inner();

    Ok(result)
}

/// The entry script sets `globalThis.defineConfig` before dynamically importing
/// the config, so configs that call `defineConfig` without an explicit import
/// work on fresh projects. Configs that do import from `./generated/index.js`
/// get an identity stub when that file is absent.
pub fn execute_config(resolver: &mut ModuleResolver, config_path: &Path) -> Result<String> {
    let canonical = config_path
        .canonicalize()
        .context("Failed to canonicalize config path")?;

    // Dynamic import runs after the globalThis assignment, so defineConfig is
    // available as a global when the config module body evaluates.
    let entry = format!(
        "globalThis.defineConfig = function(c) {{ return c; }};\nconst mod = await import({:?});\n__gha_set_config(JSON.stringify(mod.default));",
        canonical.to_string_lossy().into_owned()
    );

    let json: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    {
        let loader = resolver.clone();
        let rt = JsRuntime::new().context("Failed to create QuickJS runtime")?;
        rt.set_loader(loader.clone(), loader);
        let ctx = JsContext::full(&rt).context("Failed to create QuickJS context")?;

        ctx.with(|ctx| {
            let json_clone = json.clone();
            ctx.globals()
                .set(
                    "__gha_set_config",
                    Func::from(move |s: String| {
                        *json_clone.borrow_mut() = Some(s);
                    }),
                )
                .map_err(|e| anyhow::anyhow!("Failed to set __gha_set_config: {}", e))?;

            Module::evaluate(ctx.clone(), "__gaji_config_entry__", entry.as_str())
                .map_err(|e| anyhow::anyhow!("QuickJS config module error: {}", e))?
                .finish::<()>()
                .map_err(|e| anyhow::anyhow!("QuickJS config evaluation error: {}", e))?;

            Ok::<_, anyhow::Error>(())
        })?;
    }

    Rc::try_unwrap(json)
        .map_err(|_| anyhow::anyhow!("Failed to unwrap Rc - references still held"))?
        .into_inner()
        .context("Config did not call __gha_set_config")
}

pub fn execute_js(code: &str) -> Result<Vec<BuildOutput>> {
    let outputs: Rc<RefCell<Vec<BuildOutput>>> = Rc::new(RefCell::new(Vec::new()));

    {
        let rt = JsRuntime::new().context("Failed to create QuickJS runtime")?;
        let ctx = JsContext::full(&rt).context("Failed to create QuickJS context")?;

        let code_owned = code.to_string();

        ctx.with(|ctx| {
            let outputs_clone = outputs.clone();

            let build_fn = Func::from(
                move |id: String, json: String, output_type: rquickjs::function::Opt<String>| {
                    outputs_clone.borrow_mut().push(BuildOutput {
                        id,
                        json,
                        output_type: output_type.0.unwrap_or_else(|| "workflow".to_string()),
                    });
                },
            );

            ctx.globals()
                .set("__gha_build", build_fn)
                .map_err(|e| anyhow::anyhow!("Failed to set __gha_build: {}", e))?;

            ctx.eval::<(), _>(code_owned.as_bytes())
                .map_err(|e| anyhow::anyhow!("QuickJS evaluation error: {}", e))?;

            Ok::<_, anyhow::Error>(())
        })?;

        // ctx and rt drop here, releasing the Rc clone held by Func
    }

    // Rc::try_unwrap succeeds because ctx/rt are now dropped
    let result = Rc::try_unwrap(outputs)
        .map_err(|_| anyhow::anyhow!("Failed to unwrap Rc - references still held"))?
        .into_inner();

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_for_eval(source: &str) -> String {
        let mut result = Vec::new();
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("import ") || trimmed.starts_with("import{") {
                continue;
            }
            if trimmed.starts_with("export type ") || trimmed.starts_with("export {") {
                continue;
            }
            if trimmed.starts_with("export ") {
                result.push(trimmed.replacen("export ", "", 1));
                continue;
            }
            result.push(line.to_string());
        }
        result.join("\n")
    }

    #[test]
    fn test_strip_typescript_basic() {
        let ts_source = "const x: number = 42;\nconst y: string = \"hello\";";
        let result = strip_typescript(ts_source, "test.ts").unwrap();
        assert!(result.contains("const x = 42"));
        assert!(result.contains("const y = \"hello\""));
        assert!(!result.contains(": number"));
        assert!(!result.contains(": string"));
    }

    #[test]
    fn test_module_exports_via_import() {
        let dir = tempfile::tempdir().unwrap();
        let mod_path = dir.path().join("mod.js");
        let main_path = dir.path().join("main.js");

        std::fs::write(
            &mod_path,
            "export const x = 42;\nexport function greet() { return \"hello\"; }\n",
        )
        .unwrap();
        std::fs::write(
            &main_path,
            "import { x, greet } from './mod.js';\n\
             __gha_build('test', JSON.stringify({x, greeting: greet()}), 'workflow');\n",
        )
        .unwrap();

        let mut resolver = ModuleResolver::default();
        let outputs = execute_workflow(&mut resolver, &main_path).unwrap();
        assert_eq!(outputs.len(), 1);
        let json: serde_json::Value = serde_json::from_str(&outputs[0].json).unwrap();
        assert_eq!(json["x"], 42);
        assert_eq!(json["greeting"], "hello");
    }

    #[test]
    fn test_module_error_on_missing_dep() {
        let dir = tempfile::tempdir().unwrap();
        let mod_path = dir.path().join("test.js");
        std::fs::write(
            &mod_path,
            "import { X } from \"./nonexistent.js\";\nvar y = 1;\n",
        )
        .unwrap();

        let mut resolver = ModuleResolver::default();
        assert!(execute_workflow(&mut resolver, &mod_path).is_err());
    }

    #[test]
    fn test_module_dep_ordering() {
        let dir = tempfile::tempdir().unwrap();
        let dep_path = dir.path().join("dep.js");
        let main_path = dir.path().join("main.js");

        std::fs::write(&dep_path, "export const BASE = 10;\n").unwrap();
        std::fs::write(
            &main_path,
            "import { BASE } from './dep.js';\n\
             __gha_build('test', JSON.stringify({value: BASE + 5}), 'workflow');\n",
        )
        .unwrap();

        let mut resolver = ModuleResolver::default();
        let outputs = execute_workflow(&mut resolver, &main_path).unwrap();
        assert_eq!(outputs.len(), 1);
        let json: serde_json::Value = serde_json::from_str(&outputs[0].json).unwrap();
        assert_eq!(json["value"], 15);
    }

    #[test]
    fn test_execute_js_basic() {
        let code = r#"
            function __test() {
                __gha_build("test-workflow", '{"name":"test","on":{"push":{}},"jobs":{}}', "workflow");
            }
            __test();
        "#;
        let outputs = execute_js(code).unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].id, "test-workflow");
        assert_eq!(outputs[0].output_type, "workflow");
    }

    #[test]
    fn test_execute_js_multiple_outputs() {
        let code = r#"
            __gha_build("wf1", '{"name":"first"}', "workflow");
            __gha_build("wf2", '{"name":"second"}', "workflow");
            __gha_build("act1", '{"name":"action1"}', "action");
        "#;
        let outputs = execute_js(code).unwrap();
        assert_eq!(outputs.len(), 3);
        assert_eq!(outputs[0].output_type, "workflow");
        assert_eq!(outputs[2].output_type, "action");
    }

    #[test]
    fn test_job_workflow_pipeline() {
        use crate::generator::templates::JOB_WORKFLOW_RUNTIME_TEMPLATE;

        let runtime_js = format!(
            r#"function getAction(ref) {{
    return function(config) {{
        if (config === undefined) config = {{}};
        var step = {{ uses: ref }};
        if (config.name !== undefined) step.name = config.name;
        if (config.with !== undefined) step.with = config.with;
        return step;
    }};
}}
{}"#,
            JOB_WORKFLOW_RUNTIME_TEMPLATE
        );

        let workflow_js = r#"
var checkout = getAction("actions/checkout@v5");

new Workflow({
    name: "CI",
    on: { push: { branches: ["main"] } },
}).jobs(j => j
    .add("build",
        new Job("ubuntu-latest")
            .steps(s => s
                .add(checkout({ name: "Checkout", with: { "fetch-depth": 1 } }))
                .add({ name: "Test", run: "npm test" })
            )
    )
).build("ci");
"#;

        let runtime_stripped = strip_for_eval(&runtime_js);
        let bundled = format!("{}\n\n{}", runtime_stripped, workflow_js);

        let outputs = execute_js(&bundled).unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].id, "ci");
        assert_eq!(outputs[0].output_type, "workflow");

        let json: serde_json::Value = serde_json::from_str(&outputs[0].json).unwrap();
        assert_eq!(json["name"], "CI");
        assert!(json["on"]["push"]["branches"].is_array());
        assert_eq!(json["jobs"]["build"]["runs-on"], "ubuntu-latest");

        let steps = json["jobs"]["build"]["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0]["uses"], "actions/checkout@v5");
        assert_eq!(steps[0]["name"], "Checkout");
        assert_eq!(steps[1]["run"], "npm test");
    }

    #[test]
    fn test_composite_action_pipeline() {
        use crate::generator::templates::JOB_WORKFLOW_RUNTIME_TEMPLATE;

        let runtime_js = format!(
            "function getAction(ref) {{ return function(config) {{ return {{ uses: ref }}; }}; }}\n{}",
            JOB_WORKFLOW_RUNTIME_TEMPLATE
        );

        let workflow_js = r#"
new Action({
    name: "My Action",
    description: "A composite action",
})
    .steps(s => s
        .add({ name: "Step 1", run: "echo hello", shell: "bash" })
    )
    .build("my-action");
"#;

        let runtime_stripped = strip_for_eval(&runtime_js);
        let bundled = format!("{}\n\n{}", runtime_stripped, workflow_js);

        let outputs = execute_js(&bundled).unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].id, "my-action");
        assert_eq!(outputs[0].output_type, "action");

        let json: serde_json::Value = serde_json::from_str(&outputs[0].json).unwrap();
        assert_eq!(json["name"], "My Action");
        assert_eq!(json["runs"]["using"], "composite");
        assert_eq!(json["runs"]["steps"][0]["run"], "echo hello");
    }

    #[test]
    fn test_strip_then_execute() {
        use crate::generator::templates::JOB_WORKFLOW_RUNTIME_TEMPLATE;

        let runtime_js = format!(
            "function getAction(ref) {{ return function(config) {{ return {{ uses: ref }}; }}; }}\n{}",
            JOB_WORKFLOW_RUNTIME_TEMPLATE
        );

        let ts_source = r#"
const wf: Workflow = new Workflow({
    name: "Typed",
    on: { push: {} },
}).jobs(j => j
    .add("job1",
        new Job("ubuntu-latest")
            .steps(s => s
                .add({ name: "Hello", run: "echo hi" })
            )
    )
);

wf.build("typed-wf");
"#;

        let js = strip_typescript(ts_source, "test.ts").unwrap();

        let runtime_stripped = strip_for_eval(&runtime_js);
        let bundled = format!("{}\n\n{}", runtime_stripped, js);

        let outputs = execute_js(&bundled).unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].id, "typed-wf");

        let json: serde_json::Value = serde_json::from_str(&outputs[0].json).unwrap();
        assert_eq!(json["name"], "Typed");
        assert_eq!(json["jobs"]["job1"]["steps"][0]["run"], "echo hi");
    }
}
