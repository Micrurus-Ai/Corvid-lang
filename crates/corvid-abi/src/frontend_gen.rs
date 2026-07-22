//! React frontend scaffolding from the Application Contract
//! (slice 51q).
//!
//! `emit_react_frontend` produces a complete, runnable Vite + React +
//! TypeScript starter the developer OWNS and modifies: the generated
//! typed client (`src/corvid/`, reusing slice 51l), a configured
//! `CorvidClient`, and an `App.tsx` that wires a sign-in row plus a
//! form/stream per public agent using the `@corvid/react` components.
//! Unlike the client/hooks/components (which are shipped and reused),
//! these files are a STARTING POINT — regenerate a fresh project, then
//! edit freely; nothing here is overwritten on a re-run you don't ask
//! for.

use crate::app_contract::{ApplicationContract, ContractCallable};
use crate::ts_client::{emit_ts_client, GeneratedFile};

/// Generate the React starter project files for a contract.
pub fn emit_react_frontend(contract: &ApplicationContract) -> Vec<GeneratedFile> {
    let mut files = Vec::new();

    // The generated typed client goes under src/corvid/.
    for gf in emit_ts_client(contract) {
        files.push(GeneratedFile {
            filename: format!("src/corvid/{}", gf.filename),
            contents: gf.contents,
        });
    }

    files.push(GeneratedFile { filename: "package.json".into(), contents: PACKAGE_JSON.into() });
    files.push(GeneratedFile { filename: "tsconfig.json".into(), contents: TSCONFIG.into() });
    files.push(GeneratedFile { filename: "vite.config.ts".into(), contents: VITE_CONFIG.into() });
    files.push(GeneratedFile { filename: "index.html".into(), contents: INDEX_HTML.into() });
    files.push(GeneratedFile { filename: "src/main.tsx".into(), contents: MAIN_TSX.into() });
    files.push(GeneratedFile { filename: "src/client.ts".into(), contents: CLIENT_TS.into() });
    files.push(GeneratedFile {
        filename: "src/vite-env.d.ts".into(),
        contents: "/// <reference types=\"vite/client\" />\n".into(),
    });
    files.push(GeneratedFile { filename: "src/App.tsx".into(), contents: app_tsx(contract) });
    files.push(GeneratedFile { filename: "README.md".into(), contents: README.into() });

    files
}

fn app_tsx(contract: &ApplicationContract) -> String {
    let providers = contract
        .identities
        .iter()
        .flat_map(|i| i.providers.iter().map(|p| format!("\"{}\"", p.name)))
        .collect::<Vec<_>>()
        .join(", ");

    let mut sections = String::new();
    for agent in &contract.agents {
        sections.push_str(&agent_section(agent));
    }
    if contract.agents.is_empty() {
        sections.push_str("      <p>No public agents in this contract yet.</p>\n");
    }

    let signin = if providers.is_empty() {
        String::new()
    } else {
        format!("      <CorvidSignIn client={{client}} providers={{[{providers}]}} />\n")
    };

    format!(
        "// Starter page — you own this file; edit freely.\n\
import {{ CorvidAgentForm, CorvidStream, CorvidSignIn }} from \"@corvid/react\";\n\
import {{ client, api }} from \"./client\";\n\n\
export default function App() {{\n  \
return (\n    \
<main style={{{{ maxWidth: 720, margin: \"2rem auto\", fontFamily: \"system-ui\" }}}}>\n      \
<h1>Corvid app</h1>\n\
{signin}{sections}    </main>\n  );\n}}\n"
    )
}

fn agent_section(agent: &ContractCallable) -> String {
    let name = &agent.name;
    if agent.capabilities.streaming {
        // A streaming agent: a Start-driven stream over its first input.
        let arg = agent.inputs.first().map(|p| p.name.clone()).unwrap_or_default();
        let params = agent
            .inputs
            .iter()
            .map(|p| format!("{}: string", p.name))
            .collect::<Vec<_>>()
            .join(", ");
        let call_args = agent.inputs.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ");
        format!(
            "      <section>\n        <h2>{name}</h2>\n        <CorvidStream stream={{({params}) => api.{name}({call_args})}} args={{[\"\"]}} />\n      </section>\n      {{/* first input: {arg} */}}\n"
        )
    } else {
        let fields = agent
            .inputs
            .iter()
            .map(|p| format!("{{ name: \"{}\" }}", p.name))
            .collect::<Vec<_>>()
            .join(", ");
        let call = agent
            .inputs
            .iter()
            .map(|p| coerce_field(&p.name, &p.type_name))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "      <section>\n        <h2>{name}</h2>\n        <CorvidAgentForm fields={{[{fields}]}} call={{(v) => api.{name}({call})}} />\n      </section>\n"
        )
    }
}

/// Map a form field (always a string) into the agent's parameter type.
fn coerce_field(name: &str, ty: &str) -> String {
    if ty == "Int" || ty == "Float" {
        format!("Number(v.{name})")
    } else {
        format!("v.{name}")
    }
}

const PACKAGE_JSON: &str = r#"{
  "name": "corvid-frontend",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    "@corvid/client": "*",
    "@corvid/react": "*",
    "react": "^18.3.0",
    "react-dom": "^18.3.0"
  },
  "devDependencies": {
    "@types/react": "^18.3.0",
    "@types/react-dom": "^18.3.0",
    "@vitejs/plugin-react": "^4.3.0",
    "typescript": "^5.6.0",
    "vite": "^5.4.0"
  }
}
"#;

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM", "DOM.AsyncIterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "skipLibCheck": true,
    "noEmit": true
  },
  "include": ["src"]
}
"#;

const VITE_CONFIG: &str = r#"import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Point VITE_CORVID_BACKEND at your running `corvid serve`.
export default defineConfig({
  plugins: [react()],
});
"#;

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Corvid app</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
"#;

const MAIN_TSX: &str = r#"import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App.js";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
"#;

const CLIENT_TS: &str = r#"import { CorvidClient } from "@corvid/client";
import { Api } from "./corvid/api.js";

// Point this at your running `corvid serve` (or set VITE_CORVID_BACKEND).
const baseUrl = import.meta.env.VITE_CORVID_BACKEND ?? "http://localhost:8080";

export const client = new CorvidClient({ baseUrl });
export const api = new Api(client);
"#;

const README: &str = r#"# Corvid frontend starter

Generated by `corvid generate frontend --framework react`. You own these
files — edit them freely.

```bash
npm install
VITE_CORVID_BACKEND=http://localhost:8080 npm run dev   # point at `corvid serve`
```

- `src/corvid/` — the generated typed client (`types.ts` + `api.ts`).
  Regenerate with `corvid generate frontend` (or `corvid contract
  ts-client`) whenever the Corvid source changes.
- `src/client.ts` — the configured `CorvidClient` + `Api`.
- `src/App.tsx` — a starter page with a form per public agent and a
  sign-in row. Replace it with your product UI; the hooks
  (`@corvid/react`) are the real building blocks.
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_contract::{effect_decls_of, emit_application_contract, ContractOptions};
    use corvid_types::effects::EffectRegistry;

    fn contract_for(src: &str) -> ApplicationContract {
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, perr) = corvid_syntax::parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = corvid_resolve::resolve(&file);
        assert!(resolved.errors.is_empty(), "resolve: {:?}", resolved.errors);
        let registry = EffectRegistry::from_decls(&effect_decls_of(&file));
        let checked = corvid_types::typecheck(&file, &resolved);
        assert!(checked.errors.is_empty(), "check: {:?}", checked.errors);
        emit_application_contract(
            &file,
            &resolved,
            &checked,
            &registry,
            &ContractOptions { source_path: "app.cor", compiler_version: "test", generated_at: "now" },
        )
    }

    #[test]
    fn scaffold_emits_a_runnable_project_tree() {
        let files = emit_react_frontend(&contract_for(
            "identity app_users:
    provider google

public agent classify(question: String) -> String:
    return question

public agent chat(message: String) -> Stream<String>:
    return stream_answer(message)

tool stream_answer(m: String) -> Stream<String>
",
        ));
        let names: Vec<&str> = files.iter().map(|f| f.filename.as_str()).collect();
        for required in [
            "package.json",
            "tsconfig.json",
            "vite.config.ts",
            "index.html",
            "src/main.tsx",
            "src/client.ts",
            "src/App.tsx",
            "src/corvid/types.ts",
            "src/corvid/api.ts",
        ] {
            assert!(names.contains(&required), "missing {required}");
        }
        let app = &files.iter().find(|f| f.filename == "src/App.tsx").unwrap().contents;
        // Sign-in from the identity providers.
        assert!(app.contains("CorvidSignIn client={client} providers={[\"google\"]}"));
        // A form for the non-streaming agent, a stream for the streaming one.
        assert!(app.contains("call={(v) => api.classify(v.question)}"));
        assert!(app.contains("stream={(message: string) => api.chat(message)}"));
    }

    #[test]
    fn numeric_fields_are_coerced() {
        let files = emit_react_frontend(&contract_for(
            "public agent score(n: Int) -> Int:
    return n
",
        ));
        let app = &files.iter().find(|f| f.filename == "src/App.tsx").unwrap().contents;
        assert!(app.contains("api.score(Number(v.n))"));
    }
}
