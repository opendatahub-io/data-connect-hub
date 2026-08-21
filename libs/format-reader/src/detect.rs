#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Compression {
    None,
    Gzip,
    Zstd,
}

impl Compression {
    pub fn detect(media_type: Option<&str>, format_hint: Option<&str>) -> Self {
        if let Some(hint) = format_hint {
            let lower = hint.to_lowercase();
            if lower.ends_with(".gz") || lower.ends_with(".gzip") || lower == "gz" || lower == "gzip" {
                return Compression::Gzip;
            }
            if lower.ends_with(".zst") || lower.ends_with(".zstd") || lower == "zst" || lower == "zstd" {
                return Compression::Zstd;
            }
        }

        if let Some(mt) = media_type {
            let mt_lower = mt.to_lowercase();
            if mt_lower.contains("gzip") || mt_lower.ends_with("+gz") {
                return Compression::Gzip;
            }
            if mt_lower.contains("zstd") || mt_lower.contains("zstandard") {
                return Compression::Zstd;
            }
        }

        Compression::None
    }
}

fn strip_compression_suffix(s: &str) -> &str {
    for suffix in [".gz", ".gzip", ".zst", ".zstd"] {
        if let Some(base) = s.strip_suffix(suffix) {
            return base;
        }
    }
    s
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileFormat {
    Parquet,
    Csv,
    JsonLines,
    Binary,
}

impl FileFormat {
    pub fn detect(query: &str, format_hint: Option<&str>, media_type: Option<&str>) -> Self {
        if let Some(fmt) = format_hint {
            let fmt_lower = fmt.to_lowercase();
            let base = strip_compression_suffix(&fmt_lower);
            match base {
                "parquet" => return FileFormat::Parquet,
                "csv" => return FileFormat::Csv,
                "jsonl" | "ndjson" | "jsonlines" => return FileFormat::JsonLines,
                "binary" | "raw" => return FileFormat::Binary,
                _ => {},
            }
        }

        if let Some(mt) = media_type {
            let mt_lower = mt.to_lowercase();
            if mt_lower.contains("parquet") {
                return FileFormat::Parquet;
            } else if mt_lower.contains("csv") {
                return FileFormat::Csv;
            } else if mt_lower.contains("json") {
                return FileFormat::JsonLines;
            }
        }

        let query_lower = query.to_lowercase();
        let lower = strip_compression_suffix(&query_lower);
        if lower.ends_with(".parquet") {
            FileFormat::Parquet
        } else if lower.ends_with(".csv") {
            FileFormat::Csv
        } else if lower.ends_with(".jsonl") || lower.ends_with(".ndjson") || lower.ends_with(".jsonlines") {
            FileFormat::JsonLines
        } else {
            FileFormat::Binary
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_from_hint() {
        assert_eq!(FileFormat::detect("any", Some("parquet"), None), FileFormat::Parquet);
        assert_eq!(FileFormat::detect("any", Some("csv"), None), FileFormat::Csv);
        assert_eq!(FileFormat::detect("any", Some("jsonl"), None), FileFormat::JsonLines);
        assert_eq!(FileFormat::detect("any", Some("binary"), None), FileFormat::Binary);
    }

    #[test]
    fn test_detect_from_media_type() {
        assert_eq!(
            FileFormat::detect("any", None, Some("application/vnd.apache.parquet")),
            FileFormat::Parquet
        );
        assert_eq!(FileFormat::detect("any", None, Some("text/csv")), FileFormat::Csv);
        assert_eq!(
            FileFormat::detect("any", None, Some("application/x-ndjson")),
            FileFormat::JsonLines
        );
    }

    #[test]
    fn test_detect_from_extension() {
        assert_eq!(FileFormat::detect("data.parquet", None, None), FileFormat::Parquet);
        assert_eq!(FileFormat::detect("data.csv", None, None), FileFormat::Csv);
        assert_eq!(FileFormat::detect("data.jsonl", None, None), FileFormat::JsonLines);
        assert_eq!(FileFormat::detect("data.ndjson", None, None), FileFormat::JsonLines);
    }

    #[test]
    fn test_detect_fallback_binary() {
        assert_eq!(FileFormat::detect("myrepo:latest", None, None), FileFormat::Binary);
    }

    #[test]
    fn test_hint_takes_priority() {
        assert_eq!(
            FileFormat::detect("data.csv", Some("parquet"), Some("text/csv")),
            FileFormat::Parquet
        );
    }

    #[test]
    fn test_detect_strips_compression_from_hint() {
        assert_eq!(FileFormat::detect("any", Some("csv.gz"), None), FileFormat::Csv);
        assert_eq!(
            FileFormat::detect("any", Some("parquet.zst"), None),
            FileFormat::Parquet
        );
        assert_eq!(
            FileFormat::detect("any", Some("jsonl.gzip"), None),
            FileFormat::JsonLines
        );
    }

    #[test]
    fn test_detect_path_with_colon() {
        assert_eq!(
            FileFormat::detect("logs/2026-08-21T19:00:00.csv", None, None),
            FileFormat::Csv
        );
    }

    #[test]
    fn test_detect_strips_compression_from_query() {
        assert_eq!(FileFormat::detect("data.csv.gz", None, None), FileFormat::Csv);
        assert_eq!(FileFormat::detect("data.parquet.zst", None, None), FileFormat::Parquet);
    }

    #[test]
    fn test_compression_detect_from_hint() {
        assert_eq!(Compression::detect(None, Some("csv.gz")), Compression::Gzip);
        assert_eq!(Compression::detect(None, Some("parquet.zst")), Compression::Zstd);
        assert_eq!(Compression::detect(None, Some("csv")), Compression::None);
    }

    #[test]
    fn test_compression_detect_from_media_type() {
        assert_eq!(
            Compression::detect(Some("application/vnd.oci.image.layer.v1.tar+gzip"), None),
            Compression::Gzip
        );
        assert_eq!(Compression::detect(Some("application/zstd"), None), Compression::Zstd);
        assert_eq!(
            Compression::detect(Some("application/vnd.apache.parquet"), None),
            Compression::None
        );
    }
}
