use sha2::{Digest, Sha256};

mod chunk;
mod embedders;
mod index;
mod loaders;
mod types;

pub use chunk::{chunk_document, chunk_document_with_config};
pub use embedders::{OllamaEmbedder, OpenAiEmbedder, OLLAMA_EMBEDDING_BASE, OPENAI_EMBEDDING_BASE};
pub use index::RagSqliteIndex;
pub use loaders::{document_from_text, load_html, load_markdown, load_pdf};
pub use types::{
    EmbedderConfig, EmbeddingVector, RagChunk, RagChunkingConfig, RagDocument, RagEmbedder,
    RagEmbeddingRecord, RagSearchHit,
};

pub(crate) fn validate_embeddings(
    mut vectors: Vec<EmbeddingVector>,
    count: usize,
) -> Result<Vec<EmbeddingVector>, crate::errors::RuntimeError> {
    if vectors.len() != count {
        return Err(crate::errors::RuntimeError::Other(format!(
            "expected {count} embedding vectors, received {}",
            vectors.len()
        )));
    }
    vectors.sort_by_key(|v| v.index);
    let dimension = vectors.first().map_or(0, |v| v.values.len());
    for (index, vector) in vectors.iter().enumerate() {
        if vector.index != index
            || dimension == 0
            || vector.values.len() != dimension
            || vector.values.iter().any(|n| !n.is_finite())
        {
            return Err(crate::errors::RuntimeError::Other(format!(
                "invalid embedding vector at input {index}"
            )));
        }
    }
    Ok(vectors)
}

pub(super) fn stable_id(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("doc_{}", encode_hex(&hasher.finalize()[..8]))
}

pub(super) fn encode_hex(bytes: &[u8]) -> String {
    corvid_hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn lexical_search_ranks_all_matching_chunks_before_applying_limit() {
        let mut index = RagSqliteIndex::open_in_memory().unwrap();
        for n in 0..141 {
            let text = if n < 70 {
                "blue"
            } else if n < 140 {
                "green"
            } else {
                "blue green"
            };
            let doc =
                document_from_text(&format!("doc{n:03}"), "notes", "text/plain", text).unwrap();
            index
                .replace_document(&doc, &chunk_document(&doc, 40, 0).unwrap(), &[])
                .unwrap();
        }
        assert_eq!(
            index.search_terms("green blue", 1).unwrap()[0].text,
            "blue green"
        );
        assert_eq!(index.search_terms("green blue", 150).unwrap().len(), 141);
    }

    #[test]
    fn trimmed_chunks_advance_across_a_wide_gap() {
        let doc = document_from_text("gap", "notes", "text/plain", "a     b").unwrap();
        let chunks = chunk_document(&doc, 5, 1).unwrap();
        assert_eq!(
            chunks
                .iter()
                .map(|c| (c.text.as_str(), c.start_char))
                .collect::<Vec<_>>(),
            [("a", 0), ("b", 6)]
        );
    }

    #[test]
    fn embedding_batches_keep_the_input_mapping_and_shape() {
        let vector = |index, values| EmbeddingVector { index, values };
        let ordered =
            validate_embeddings(vec![vector(1, vec![2.0]), vector(0, vec![1.0])], 2).unwrap();
        assert_eq!(ordered[0].values, [1.0]);
        for invalid in [
            vec![vector(0, vec![1.0])],
            vec![vector(1, vec![1.0]), vector(1, vec![1.0])],
            vec![vector(0, vec![1.0]), vector(1, vec![0.0, 1.0])],
            vec![vector(0, vec![f32::INFINITY]), vector(1, vec![1.0])],
        ] {
            assert!(validate_embeddings(invalid, 2).is_err());
        }
    }

    #[test]
    fn chunk_document_preserves_provenance_windows() {
        let doc = document_from_text("doc1", "memory", "text/plain", "abcdefghij").unwrap();
        let chunks = chunk_document(&doc, 4, 1).unwrap();

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].text, "abcd");
        assert_eq!(chunks[1].text, "defg");
        assert_eq!(chunks[2].text, "ghij");
        assert!(chunks[0].provenance_key.starts_with("rag_"));
        assert_ne!(chunks[0].provenance_key, chunks[1].provenance_key);
    }

    #[test]
    fn chunk_document_prefers_sentence_boundaries_and_trims_whitespace() {
        let doc = document_from_text(
            "doc1",
            "memory",
            "text/plain",
            "Alpha sentence.  Beta sentence.\nGamma sentence.",
        )
        .unwrap();
        let chunks = chunk_document_with_config(&doc, &RagChunkingConfig::new(20, 4)).unwrap();

        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].text, "Alpha sentence.");
        assert!(!chunks[1].text.starts_with(' '));
    }

    #[test]
    fn embedder_configs_cover_openai_and_ollama() {
        let openai = EmbedderConfig::openai("text-embedding-3-small");
        assert_eq!(openai.provider, "openai");
        assert_eq!(openai.endpoint, None);

        let ollama = EmbedderConfig::ollama("nomic-embed-text", "http://localhost:11434");
        assert_eq!(ollama.provider, "ollama");
        assert_eq!(ollama.endpoint.as_deref(), Some("http://localhost:11434"));
    }

    #[test]
    fn sqlite_index_round_trips_chunks_with_provenance() {
        let doc = document_from_text("doc1", "memory", "text/plain", "alpha beta gamma").unwrap();
        let chunks = chunk_document(&doc, 8, 0).unwrap();
        let mut index = RagSqliteIndex::open_in_memory().unwrap();
        index.insert_document(&doc).unwrap();
        index.insert_chunks(&chunks).unwrap();

        let hits = index.search_text("alpha", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].doc_id, "doc1");
        assert!(hits[0].provenance_key.starts_with("rag_"));
    }

    #[test]
    fn sqlite_index_returns_grounded_hits() {
        let doc = document_from_text("doc1", "memory", "text/plain", "alpha beta gamma").unwrap();
        let chunks = chunk_document(&doc, 8, 0).unwrap();
        let mut index = RagSqliteIndex::open_in_memory().unwrap();
        index.insert_document(&doc).unwrap();
        index.insert_chunks(&chunks).unwrap();

        let hits = index.search_grounded_text("alpha", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].has_retrieval());
        assert_eq!(hits[0].value.doc_id, "doc1");
    }

    #[test]
    fn sqlite_index_searches_embeddings_by_similarity() {
        let doc =
            document_from_text("doc1", "memory", "text/plain", "alpha beta gamma delta").unwrap();
        let chunks = chunk_document(&doc, 6, 0).unwrap();
        let mut index = RagSqliteIndex::open_in_memory().unwrap();
        index.insert_document(&doc).unwrap();
        index.insert_chunks(&chunks).unwrap();
        index
            .insert_embedding_vectors(&[
                RagEmbeddingRecord {
                    chunk_id: chunks[0].chunk_id.clone(),
                    values: vec![1.0, 0.0],
                },
                RagEmbeddingRecord {
                    chunk_id: chunks[1].chunk_id.clone(),
                    values: vec![0.8, 0.2],
                },
                RagEmbeddingRecord {
                    chunk_id: chunks[2].chunk_id.clone(),
                    values: vec![0.0, 1.0],
                },
            ])
            .unwrap();

        let hits = index.search_embeddings(&[0.9, 0.1], 2).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits[0].score >= hits[1].score);
        assert_eq!(hits[0].chunk.chunk_id, chunks[0].chunk_id);

        let grounded = index
            .search_grounded_embeddings("alpha-ish", &[0.9, 0.1], 1)
            .unwrap();
        assert_eq!(grounded.len(), 1);
        assert!(grounded[0].has_retrieval());
        assert_eq!(grounded[0].value.chunk_id, chunks[0].chunk_id);
    }

    #[test]
    fn load_html_extracts_readable_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.html");
        std::fs::write(
            &path,
            "<html><head><style>.x{color:red}</style></head><body><h1>Title</h1><p>Hello <b>world</b> &amp; friends</p><script>ignored()</script><div>Next line</div></body></html>",
        )
        .unwrap();

        let doc = load_html(&path).unwrap();

        assert_eq!(doc.media_type, "text/html");
        assert_eq!(doc.text, "Title\nHello world & friends\nNext line");
    }

    #[test]
    fn load_pdf_extracts_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.pdf");
        std::fs::write(&path, minimal_pdf_bytes("Hello PDF")).unwrap();

        let doc = load_pdf(&path).unwrap();

        assert_eq!(doc.media_type, "application/pdf");
        assert!(doc.text.contains("Hello PDF"));
    }

    #[tokio::test]
    async fn openai_embedder_posts_embedding_requests() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"index": 1, "embedding": [0.3, 0.4]},
                    {"index": 0, "embedding": [0.1, 0.2]}
                ]
            })))
            .mount(&server)
            .await;

        let embedder =
            OpenAiEmbedder::new("test-key", "text-embedding-3-small").with_base_url(server.uri());
        let texts = vec!["alpha".to_string(), "beta".to_string()];
        let embeddings = embedder.embed(&texts).await.unwrap();

        assert_eq!(embedder.provider(), "openai");
        assert_eq!(embedder.model(), "text-embedding-3-small");
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].index, 0);
        assert_eq!(embeddings[0].values, vec![0.1, 0.2]);
        assert_eq!(embeddings[1].index, 1);
    }

    #[tokio::test]
    async fn ollama_embedder_reads_batch_embeddings() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/embed"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "embeddings": [
                    [0.5, 0.6],
                    [0.7, 0.8]
                ]
            })))
            .mount(&server)
            .await;

        let embedder = OllamaEmbedder::new("nomic-embed-text").with_endpoint(server.uri());
        let texts = vec!["alpha".to_string(), "beta".to_string()];
        let embeddings = embedder.embed(&texts).await.unwrap();

        assert_eq!(embedder.provider(), "ollama");
        assert_eq!(embedder.model(), "nomic-embed-text");
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].values, vec![0.5, 0.6]);
        assert_eq!(embeddings[1].values, vec![0.7, 0.8]);
    }

    fn minimal_pdf_bytes(text: &str) -> Vec<u8> {
        let content = format!(
            "BT\n/F1 24 Tf\n72 72 Td\n({}) Tj\nET",
            escape_pdf_text(text)
        );
        let objects = vec![
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 144] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n".to_string(),
            format!(
                "4 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                content.len(),
                content
            ),
            "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n"
                .to_string(),
        ];
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = vec![0usize];
        for object in &objects {
            offsets.push(pdf.len());
            pdf.push_str(object);
        }
        let xref_offset = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n", objects.len() + 1));
        pdf.push_str("0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_offset
        ));
        pdf.into_bytes()
    }

    fn escape_pdf_text(text: &str) -> String {
        text.replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)")
    }
}
