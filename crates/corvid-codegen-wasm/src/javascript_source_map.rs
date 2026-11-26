use base64::{engine::general_purpose::STANDARD, Engine};

/// One mapping per generated line. Runtime glue between handlers stays unmapped.
#[derive(Default)]
pub(super) struct JavascriptSourceMap {
    sources: Vec<String>,
    mappings: String,
    generated_line: usize,
    source_index: i64,
    original_line: i64,
}

impl JavascriptSourceMap {
    pub(super) fn record(&mut self, generated_line: usize, location: Option<String>) {
        let Some(location) = location else { return };
        let Some((path_line, _)) = location.rsplit_once(':') else {
            return;
        };
        let Some((path, line)) = path_line.rsplit_once(':') else {
            return;
        };
        let Ok(line) = line.parse::<i64>() else {
            return;
        };
        let index = self
            .sources
            .iter()
            .position(|source| source == path)
            .unwrap_or_else(|| {
                self.sources.push(path.to_owned());
                self.sources.len() - 1
            }) as i64;
        for _ in self.generated_line..generated_line {
            self.mappings.push(';');
        }
        self.mappings.push('A'); // Generated column zero.
        write_delta(&mut self.mappings, index - self.source_index);
        write_delta(&mut self.mappings, line - 1 - self.original_line);
        self.mappings.push('A'); // Original column zero.
        self.generated_line = generated_line;
        self.source_index = index;
        self.original_line = line - 1;
    }

    pub(super) fn append_to(self, module: &mut String) {
        if self.sources.is_empty() {
            return;
        }
        let map = serde_json::json!({
            "version": 3, "sources": self.sources, "names": [], "mappings": self.mappings
        });
        module.push_str("\n//# sourceMappingURL=data:application/json;base64,");
        module.push_str(&STANDARD.encode(map.to_string()));
        module.push('\n');
    }
}

fn write_delta(output: &mut String, delta: i64) {
    const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut value = (delta.unsigned_abs() << 1) | u64::from(delta < 0);
    loop {
        let digit = value & 31;
        value >>= 5;
        let continuation = if value == 0 { 0 } else { 32 };
        output.push(DIGITS[(digit | continuation) as usize] as char);
        if value == 0 {
            break;
        }
    }
}
