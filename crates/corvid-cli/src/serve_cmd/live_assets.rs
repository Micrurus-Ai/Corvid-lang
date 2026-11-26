//! Browser modules used by Corvid live views. The SDK modules are TypeScript
//! compiler output, checked byte-for-byte by verify_generated_live_client.py.
use axum::body::Bytes;
use corvid_abi::app_contract::ContractLive;

pub(super) fn for_contract(
    live: &[ContractLive],
) -> serde_json::Result<Vec<(&'static str, &'static str, Bytes)>> {
    let mut assets = Vec::new();
    if live.is_empty() {
        return Ok(assets);
    }
    let mime = "text/javascript; charset=utf-8";
    macro_rules! sdk_module {
        ($name:literal) => {
            assets.push((
                $name,
                mime,
                Bytes::from_static(include_bytes!(concat!(
                    "../../../../sdk/typescript/client/browser/",
                    $name
                ))),
            ))
        };
    }
    sdk_module!("errors.js");
    sdk_module!("live-rows.js");
    sdk_module!("path.js");
    sdk_module!("live-connection.js");
    let mut module = String::from(include_str!(
        "../../../corvid-codegen-wasm/src/row_order.js"
    ));
    module.push_str(include_str!("live-element.js"));
    module.push_str("\nconst liveDeclarations = new Map(");
    module.push_str(&serde_json::to_string(live)?);
    module.push_str(".map(route => [JSON.stringify([route.server, route.name]), route]));\n");
    module.push_str("customElements.define('corvid-live', CorvidLiveList);\n");
    assets.push(("live-element.js", mime, Bytes::from(module)));
    Ok(assets)
}
