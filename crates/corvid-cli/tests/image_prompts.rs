//! A local model inspects the actual image part sent by an ordinary Corvid prompt.
use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use std::{
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAAAAAA6fptVAAAACklEQVR4nGP4DwABAQEAsTj2FAAAAABJRU5ErkJggg==";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_file_reaches_typed_prompt_as_image_content() {
    async fn chat(
        State(count): State<Arc<AtomicUsize>>,
        uri: axum::http::Uri,
        Json(req): Json<Value>,
    ) -> Json<Value> {
        count.fetch_add(1, Ordering::SeqCst);
        let user = &req["messages"][0];
        let response = json!({"label":"one pixel"}).to_string();
        if uri.path() == "/api/chat" {
            assert_eq!(user["images"], json!([PNG]));
            assert!(user["content"].as_str().unwrap().contains("describe"));
            Json(json!({"message":{"role":"assistant","content":response},"done":true}))
        } else {
            let parts = user["content"].as_array().unwrap();
            assert!(parts[0]["text"].as_str().unwrap().contains("describe"));
            assert!(!parts[0]["text"].as_str().unwrap().contains(PNG));
            assert_eq!(
                parts[1]["image_url"]["url"],
                format!("data:image/png;base64,{PNG}")
            );
            Json(json!({"choices":[{"message":{"role":"assistant","content":response}}]}))
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/api/chat", post(chat))
        .route("/v1/chat/completions", post(chat))
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    let bin = env!("CARGO_BIN_EXE_corvid");
    assert!(Command::new(bin)
        .current_dir(tmp.path())
        .args(["new", "photos"])
        .output()
        .unwrap()
        .status
        .success());
    let project = tmp.path().join("photos");
    let path = project.join("src/main.cor");
    let code = r#"import "std:io" use PromptImage, io_read_image

type Caption:
    label: String

prompt describe(photo: PromptImage) -> Caption:
    "Please describe {photo}."

agent inspect() -> Result<String, String>:
    photo = io_read_image("photo.png")?
    caption = describe(photo)
    return Ok(caption.label)

agent main() -> String:
    return match inspect():
        Ok(label) -> label
        Err(message) -> message
"#;
    std::fs::write(&path, code).unwrap();
    let run = |model: &str| {
        let out = Command::new(bin)
            .current_dir(&project)
            .args(["run", "src/main.cor"])
            .env("CORVID_IO_ROOT", &project)
            .env("CORVID_MODEL", model)
            .env("OLLAMA_BASE_URL", &base)
            .env("CORVID_TEST_MOCK_LLM", "0")
            .env("OPENAI_COMPAT_API_KEY", "")
            .env("NO_PROXY", "127.0.0.1")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    let compat = format!("openai-compat:{base}/v1:vision");
    std::fs::write(
        project.join("photo.png"),
        include_bytes!("fixtures/prompt_pixel.png"),
    )
    .unwrap();
    for model in [&*compat, "ollama:vision"] {
        assert!(run(model).contains("one pixel"));
    }
    std::fs::write(project.join("photo.png"), "plain text").unwrap();
    assert!(run(&compat).contains("PNG or JPEG"));
    std::fs::write(
        tmp.path().join("outside.png"),
        include_bytes!("fixtures/prompt_pixel.png"),
    )
    .unwrap();
    std::fs::write(&path, code.replace("photo.png", "../outside.png")).unwrap();
    assert!(run(&compat).contains("outside the configured I/O root"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Playwright and an installed Chromium browser"]
async fn scoped_browser_session_reads_clicks_captures_and_closes() {
    use axum::{response::Html, routing::get};
    let seen = Arc::new(AtomicUsize::new(0));
    let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let outside = format!("http://{}/away", other.local_addr().unwrap());
    let counter = seen.clone();
    let foreign = tokio::spawn(async move {
        axum::serve(
            other,
            Router::new().route(
                "/away",
                get(move || async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    "other page"
                }),
            ),
        )
        .await
        .unwrap()
    });
    let html = format!(
        r#"<p id="state">before</p><button onclick="document.querySelector('#state').textContent='after'">Update</button><button onclick="location.href='{outside}'">Leave</button><button onclick="location.href='/redirect'">Redirect</button>"#
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}/", listener.local_addr().unwrap());
    let site = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/", get(move || async move { Html(html) }))
                .route("/redirect", get(move || async move { axum::response::Redirect::temporary(&outside) }))
                .route("/v1/chat/completions",post(|Json(req):Json<Value>| async move {
                    let url=req["messages"][0]["content"][1]["image_url"]["url"].as_str().unwrap();
                    assert!(url.starts_with("data:image/png;base64,iVBOR") && url.len()>500);
                    Json(json!({"choices":[{"message":{"role":"assistant","content":"received capture"}}]}))
                })),
        )
        .await
        .unwrap()
    });
    let tmp = tempfile::tempdir().unwrap();
    let bin = env!("CARGO_BIN_EXE_corvid");
    assert!(Command::new(bin)
        .current_dir(tmp.path())
        .args(["new", "session"])
        .output()
        .unwrap()
        .status
        .success());
    let dir = tmp.path().join("session");
    let run = |body: &str| {
        let source = r#"import "std:browser" use browser_open_scoped, browser_close, browser_click, browser_read_text, browser_capture
import "std:io" use io_read_image, PromptImage

prompt inspect_capture(photo: PromptImage) -> String:
    "Describe the page."

agent main() -> String:
BODY
"#;
        std::fs::write(dir.join("src/main.cor"), source.replace("BODY", body)).unwrap();
        Command::new(bin)
            .current_dir(&dir)
            .args(["run", "src/main.cor"])
            .env("CORVID_BROWSER_URL", &address)
            .env("CORVID_IO_ROOT", &dir)
            .env("CORVID_MODEL", format!("openai-compat:{address}v1:vision"))
            .env("CORVID_TEST_MOCK_LLM", "0")
            .env("OPENAI_COMPAT_API_KEY", "")
            .env("NO_PROXY", "127.0.0.1")
            .env("CORVID_BROWSER_EVIDENCE_DIR", dir.join("captures"))
            .output()
            .unwrap()
    };
    let result = run(r##"    browser_open_scoped(480, 320)
    browser_click("Update")
    changed = browser_read_text("#state")
    capture = browser_capture()
    browser_close()
    browser_open_scoped(480, 320)
    fresh = browser_read_text("#state")
    browser_close()
    photo = io_read_image(capture)
    return match photo:
        Ok(image) -> changed + "/" + fresh + "/" + inspect_capture(image)
        Err(message) -> message"##);
    assert!(
        result.status.success()
            && String::from_utf8_lossy(&result.stdout).contains("after/before/received capture"),
        "{result:?}"
    );
    let folder = std::fs::read_dir(dir.join("captures"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(std::fs::read(folder.join("capture-01.png"))
        .unwrap()
        .starts_with(b"\x89PNG"));
    let denied = run(r#"    browser_open_scoped(480, 320)
    browser_click("Leave")
    return browser_read_text("body")"#);
    assert!(!denied.status.success(), "{denied:?}");
    let redirect = run(r#"    browser_open_scoped(480, 320)
    browser_click("Redirect")
    return browser_read_text("body")"#);
    assert!(!redirect.status.success(), "{redirect:?}");
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    foreign.abort();
    site.abort();
}
