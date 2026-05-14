//! `corvid tour` — the native CLI rendering of the demo catalog.
//!
//! The catalog data itself (`TourTopic` + `TOPICS` + `find_topic`)
//! lives in the wasm-clean `corvid-tour-catalog` crate so the
//! browser playground can render the same curated set. This module
//! is the native presentation layer: text rendering for the
//! terminal plus the REPL hand-off.

use anyhow::{anyhow, Result};

use corvid_tour_catalog::{find_topic, TourTopic, TOPICS};

pub fn cmd_tour(list: bool, topic: Option<&str>) -> Result<u8> {
    if list || topic.is_none() {
        print!("{}", render_tour_list());
        return Ok(0);
    }
    let topic_name = topic.unwrap();
    let topic = find_topic(topic_name)
        .ok_or_else(|| anyhow!("unknown tour topic `{topic_name}`; run `corvid tour --list`"))?;
    print!("{}", render_topic_card(topic));
    corvid_repl::Repl::run_tour_stdio(topic.title, topic.source)?;
    Ok(0)
}

pub fn render_tour_list() -> String {
    let mut out = String::new();
    out.push_str("Corvid invention tour\n\n");
    for topic in TOPICS {
        out.push_str(&format!(
            "  {:<22} {:<24} {}\n",
            topic.name, topic.category, topic.title
        ));
    }
    out.push_str("\nRun `corvid tour --topic <name>` to load a demo into the REPL.\n");
    out
}

pub fn render_topic_card(topic: &TourTopic) -> String {
    format!(
        "Topic: {title}\nCategory: {category}\n\n{pitch}\n\nSpec: {spec}\nRoadmap: {roadmap}\nTest: {test}\nNon-scope: {non_scope}\n\n",
        title = topic.title,
        category = topic.category,
        pitch = topic.pitch,
        spec = topic.spec,
        roadmap = topic.roadmap,
        test = topic.test,
        non_scope = topic.non_scope,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_mentions_every_topic_name() {
        let list = render_tour_list();
        for topic in TOPICS {
            assert!(list.contains(topic.name), "missing {}", topic.name);
        }
    }

    #[test]
    fn all_tour_sources_compile() {
        for topic in TOPICS {
            let compiled = corvid_driver::compile(topic.source);
            assert!(
                compiled.ok(),
                "tour topic `{}` failed to compile: {:?}",
                topic.name,
                compiled.diagnostics
            );
        }
    }
}
