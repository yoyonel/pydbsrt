use dbsrt_core::metadata::MediaMetadata;
use dbsrt_core::providers::{
    cache::SubtitleCache, mock::MockSubtitleProvider, score_candidate, OpenSubtitlesProvider,
    RemoteSubtitleCandidate, SubtitleProvider,
};
use std::fs;

#[test]
fn test_heuristic_scoring_matrix() {
    let meta =
        MediaMetadata::parse("Game.of.Thrones.S08E06.1080p.AMZN.WEB-DL.DDP5.1.H.264-GoT.mkv");

    // Perfect match
    let rel_perfect = "Game.of.Thrones.S08E06.1080p.AMZN.WEB-DL-GoT";
    let score_perf = score_candidate(
        &meta,
        Some(rel_perfect),
        Some("Game of Thrones"),
        Some(23.976),
    );
    assert!(score_perf >= 0.95);

    // Partial match: BluRay KILLERS (mismatched group and source)
    let rel_mismatch = "Game.of.Thrones.S08E06.720p.HDTV.x264-KILLERS";
    let score_mis = score_candidate(&meta, Some(rel_mismatch), Some("Game of Thrones"), None);
    assert_eq!(score_mis, 0.1);

    // Moderate match: same source WEB-DL and resolution 1080p, different group
    let rel_mod = "Game.of.Thrones.S08E06.1080p.WEB-DL-FLUX";
    let score_mod = score_candidate(&meta, Some(rel_mod), Some("Game of Thrones"), None);
    // 0.1 + 0.3 (source) + 0.2 (res) = 0.6
    assert!((score_mod - 0.6).abs() < 0.01);
}

#[test]
fn test_cache_naming_and_persistence() {
    let tmp_dir = std::env::temp_dir().join(format!(
        "dbsrt_cache_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    let cache = SubtitleCache::new(Some(tmp_dir.clone()));

    // TV episode metadata
    let tv_meta = MediaMetadata::parse("Breaking.Bad.S05E16.720p.HDTV.x264-ASAP.mkv");
    let tv_srt = "1\n00:00:01,000 --> 00:00:02,000\nFelina\n";

    let tv_path = cache.put(&tv_meta, "en", "101", tv_srt).expect("put tv");
    assert!(tv_path.to_string_lossy().contains("Breaking_Bad"));
    assert!(tv_path.to_string_lossy().contains("s05e16"));
    assert!(tv_path.to_string_lossy().contains("en.101.srt"));

    assert!(cache.exists(&tv_meta, "en", "101"));
    assert_eq!(cache.get(&tv_meta, "en", "101"), Some(tv_srt.to_string()));

    // Movie metadata
    let movie_meta = MediaMetadata::parse("Inception.2010.1080p.BluRay.x264.mkv");
    let movie_srt = "1\n00:00:10,000 --> 00:00:12,000\nAn idea...\n";

    let movie_path = cache
        .put(&movie_meta, "fr", "202", movie_srt)
        .expect("put movie");
    assert!(movie_path.to_string_lossy().contains("Inception"));
    assert!(movie_path.to_string_lossy().contains("2010"));
    assert!(movie_path.to_string_lossy().contains("fr.202.srt"));

    assert!(cache.exists(&movie_meta, "fr", "202"));
    assert_eq!(
        cache.get(&movie_meta, "fr", "202"),
        Some(movie_srt.to_string())
    );

    let _ = fs::remove_dir_all(&tmp_dir);
}

#[tokio::test]
async fn test_mock_provider_ranking_and_retrieval() {
    let provider = MockSubtitleProvider::new();
    let meta = MediaMetadata::parse("Dark.S01E01.1080p.NF.WEB-DL-STRiFE.mkv");

    let cand_best = RemoteSubtitleCandidate {
        provider: "mock".to_string(),
        subtitle_id: "best-1".to_string(),
        release_name: Some("Dark.S01E01.1080p.NF.WEB-DL-STRiFE".to_string()),
        language: "de".to_string(),
        download_url: "mock://best-1".to_string(),
        hearing_impaired: false,
        fps: Some(25.0),
        match_score: 0.95,
    };

    let cand_low = RemoteSubtitleCandidate {
        provider: "mock".to_string(),
        subtitle_id: "low-2".to_string(),
        release_name: Some("Dark.S01E01.HDTV-XviD".to_string()),
        language: "de".to_string(),
        download_url: "mock://low-2".to_string(),
        hearing_impaired: true,
        fps: None,
        match_score: 0.2,
    };

    let srt_best = "1\n00:00:01,000 --> 00:00:03,000\nWo ist Mikkel?\n";
    let srt_low = "1\n00:00:01,000 --> 00:00:03,000\n(Mikkel schreit)\n";

    provider.add_candidate("dark", cand_best.clone(), srt_best);
    provider.add_candidate("dark", cand_low.clone(), srt_low);

    let found = provider.search(&meta, "de").await.expect("search de");
    assert_eq!(found.len(), 2);

    let downloaded = provider.download(&found[0]).await.expect("download");
    assert_eq!(downloaded, srt_best);
}

#[test]
fn test_opensubtitles_provider_builder() {
    let os = OpenSubtitlesProvider::new(None)
        .with_base_url("https://custom.api.opensubtitles.com/api/v1")
        .with_api_key("test_key");

    assert_eq!(os.name(), "opensubtitles");
}
