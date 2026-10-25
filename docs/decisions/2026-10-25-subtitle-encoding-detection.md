# ADR: Subtitle Character Encoding Detection

- Date: 2026-10-25
- Statut: Accepté (post-merge bug fix)
- Contexte: Bug post-merge sur Euphoria S01E02 (Latin-1) — accents français corrompus en U+FFFD dans VLC. Cause racine: `String::from_utf8_lossy` sur bytes Windows-1252 / ISO-8859-1.
- Décision: 3-tier decoding via `encoding_rs` + `chardetng`
  1. BOM detection (`encoding_rs::Encoding::for_bom`) pour UTF-8, UTF-16LE, UTF-16BE
  2. UTF-8 strict fast-path (`std::str::from_utf8`, zero-copy)
  3. Statistical detection (`chardetng`, Mozilla engine utilisé par Firefox)
- Conséquences:
  - +2 dependencies (`encoding_rs`, `chardetng` — Mozilla-maintained, stables, sans unsafe lourd)
  - Zero-copy pour UTF-8 (90%+ des SRTs modernes)
  - ~1ms détection statistique pour fichiers legacy (acceptable)
  - Tous les read paths normalisés via `decode_subtitle_bytes()`
- Sites mis à jour:
  - `crates/dbsrt-core/src/srt.rs` (`SubtitleFile::open()`, `SubtitleFile::from_bytes()`)
  - `crates/dbsrt-core/src/container.rs` (`extract_embedded_subtitle()`)
  - `crates/dbsrt-core/src/providers/opensubtitles.rs` (`download()`)
  - `crates/dbsrt-server/src/handlers/search/download.rs` (`download_handler()`)
- Tests E2E:
  - `test_decode_subtitle_bytes_windows1252_accents`
  - `test_decode_subtitle_bytes_utf8_bom`
  - `test_decode_subtitle_bytes_utf16le_bom`
  - `test_open_euphoria_latin1_srt`
  - `test_download_latin1_subtitle_preserves_french_accents_utf8`
