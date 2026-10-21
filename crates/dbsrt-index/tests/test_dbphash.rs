//! Unit and integration test suite for unified .dbphash format (v1.0).

use dbsrt_core::metadata::{MediaMetadata, MediaType};
use dbsrt_index::dbphash::{DbphashBuilder, MmapDbphash, DBPHASH_MAGIC, DBPHASH_VERSION};
use std::fs;

#[test]
fn test_dbphash_pack_and_mmap_search() {
    let tmp_dir = std::env::temp_dir();
    let dbphash_path = tmp_dir.join(format!("test_catalog_{}.dbphash", std::process::id()));

    // 1. Build test database with 3 media items
    let mut builder = DbphashBuilder::new();

    let meta1 = MediaMetadata {
        raw_title: "Silicon.Valley.S01E01.1080p.mkv".to_string(),
        clean_title: "Silicon Valley".to_string(),
        media_type: MediaType::TvEpisode,
        year: Some(2014),
        season: Some(1),
        episode: Some(1),
        resolution: Some("1080p".to_string()),
        source: Some("WEB-DL".to_string()),
        release_group: Some("KILLERS".to_string()),
        moviehash: None,
    };
    let cuts1 = [
        (0x0123_4567_89ab_cdef_u64, 1000u32),
        (0x0123_4567_89ab_cdee_u64, 5000u32),
        (0x1111_2222_3333_4444_u64, 12000u32),
    ];
    let sparse1 = [
        (0x0123_4567_89ab_cdef_u64, 0u32),
        (0x0123_4567_89ab_cdee_u64, 1000u32),
        (0x1111_2222_3333_4444_u64, 2000u32),
    ];
    let id1 = builder.add_media(&meta1, 1_800_000, 25.0, &cuts1, &sparse1);
    assert_eq!(id1, 1);

    let meta2 = MediaMetadata {
        raw_title: "Kingdom.2019.S01E01.1080p.mkv".to_string(),
        clean_title: "Kingdom".to_string(),
        media_type: MediaType::TvEpisode,
        year: Some(2019),
        season: Some(1),
        episode: Some(1),
        resolution: Some("1080p".to_string()),
        source: Some("WEB-DL".to_string()),
        release_group: Some("MZABI".to_string()),
        moviehash: None,
    };
    let cuts2 = [
        (0xaaaa_bbbb_cccc_dddd_u64, 2000u32),
        (0xaaaa_bbbb_cccc_ddde_u64, 8000u32),
    ];
    let sparse2 = [
        (0xaaaa_bbbb_cccc_dddd_u64, 0u32),
        (0xaaaa_bbbb_cccc_ddde_u64, 1000u32),
    ];
    let id2 = builder.add_media(&meta2, 3_200_000, 23.976, &cuts2, &sparse2);
    assert_eq!(id2, 2);

    let meta3 = MediaMetadata {
        raw_title: "[SubsPlease] Sousou no Frieren - 01 (1080p).mkv".to_string(),
        clean_title: "Sousou no Frieren".to_string(),
        media_type: MediaType::Anime,
        year: Some(2023),
        season: None,
        episode: Some(1),
        resolution: Some("1080p".to_string()),
        source: None,
        release_group: Some("SubsPlease".to_string()),
        moviehash: None,
    };
    let cuts3 = [
        (0xffff_0000_ffff_0000_u64, 500u32),
        (0xffff_0000_ffff_0001_u64, 3500u32),
        (0xffff_0000_ffff_0002_u64, 9000u32),
        (0xffff_0000_ffff_0003_u64, 15000u32),
    ];
    let sparse3 = [
        (0xffff_0000_ffff_0000_u64, 0u32),
        (0xffff_0000_ffff_0001_u64, 1000u32),
        (0xffff_0000_ffff_0002_u64, 2000u32),
    ];
    let id3 = builder.add_media(&meta3, 1_440_000, 24.0, &cuts3, &sparse3);
    assert_eq!(id3, 3);

    // 2. Write to disk
    let stats = builder
        .write_to_file(&dbphash_path)
        .expect("Failed to write .dbphash");
    assert_eq!(stats.media_count, 3);
    assert_eq!(stats.total_cuts_count, 9);
    assert_eq!(stats.total_sparse_count, 8);
    assert!(stats.output_bytes >= 128 + (3 * 64) + (9 * 16) + (8 * 16));

    // 3. Open via MmapDbphash
    let mmap_db = MmapDbphash::open(&dbphash_path).expect("Failed to mmap .dbphash");

    // Header validation
    let hdr = mmap_db.header();
    assert_eq!(hdr.magic, DBPHASH_MAGIC);
    assert_eq!(hdr.version, DBPHASH_VERSION);
    assert_eq!(hdr.media_count, 3);
    assert_eq!(hdr.total_cuts_count, 9);
    assert_eq!(hdr.total_sparse_count, 8);

    // Media entries validation
    let media = mmap_db.media_entries();
    assert_eq!(media.len(), 3);

    assert_eq!(mmap_db.get_title(&media[0]), "Silicon Valley");
    assert_eq!(media[0].year, 2014);
    assert_eq!(media[0].season, 1);
    assert_eq!(media[0].episode, 1);
    assert_eq!(media[0].cuts_count, 3);
    assert_eq!(media[0].sparse_count, 3);
    assert_eq!(media[0].media_type_enum(), MediaType::TvEpisode);

    assert_eq!(mmap_db.get_title(&media[1]), "Kingdom");
    assert_eq!(media[1].year, 2019);
    assert_eq!(media[1].cuts_count, 2);
    assert_eq!(media[1].sparse_count, 2);

    assert_eq!(mmap_db.get_title(&media[2]), "Sousou no Frieren");
    assert_eq!(media[2].media_type_enum(), MediaType::Anime);
    assert_eq!(media[2].cuts_count, 4);
    assert_eq!(media[2].sparse_count, 3);

    // O(1) ID lookup
    let found = mmap_db.find_media_by_id(2).expect("Media ID 2 not found");
    assert_eq!(mmap_db.get_title(found), "Kingdom");

    // Cuts & Sparse entries validation
    let cuts = mmap_db.cuts_entries();
    assert_eq!(cuts.len(), 9);
    assert_eq!(cuts[0].phash, 0x0123_4567_89ab_cdef_u64);
    assert_eq!(cuts[0].media_id, 1);

    let sparse = mmap_db.sparse_entries();
    assert_eq!(sparse.len(), 8);
    assert_eq!(sparse[0].phash, 0x0123_4567_89ab_cdef_u64);
    assert_eq!(sparse[0].media_id, 1);

    // Search cuts
    let query = [0x0123_4567_89ab_cdef_u64];
    let matches = mmap_db.search_cuts(&query, 0);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].cut_entry.media_id, 1);
    assert_eq!(matches[0].distance, 0);

    // Search sparse
    let matches_sp = mmap_db.search_sparse(&query, 0);
    assert_eq!(matches_sp.len(), 1);
    assert_eq!(matches_sp[0].cut_entry.media_id, 1);

    // Media Recognition Ranking (Both cuts and sparse match)
    let query_batch = [
        0x0123_4567_89ab_cdef_u64, // matches media 1
        0x0123_4567_89ab_cdee_u64, // matches media 1
        0x1111_2222_3333_4444_u64, // matches media 1
    ];
    let ranked = mmap_db.recognize(&query_batch, 0);
    assert!(!ranked.is_empty());
    assert_eq!(ranked[0].clean_title, "Silicon Valley");
    assert_eq!(ranked[0].matching_cuts, 3);
    assert_eq!(ranked[0].matching_sparse, 3);
    assert_eq!(ranked[0].confidence, 100.0);

    // Cleanup
    let _ = fs::remove_file(&dbphash_path);
}

#[test]
fn test_dbphash_corrupted_header_handling() {
    let tmp_dir = std::env::temp_dir();
    let corrupt_path = tmp_dir.join(format!("corrupt_{}.dbphash", std::process::id()));

    // File too small
    fs::write(&corrupt_path, b"TOO_SHORT").unwrap();
    assert!(MmapDbphash::open(&corrupt_path).is_err());

    // Bad magic
    let mut fake_bytes = vec![0u8; 128];
    fake_bytes[..8].copy_from_slice(b"BADMAGIC");
    fs::write(&corrupt_path, &fake_bytes).unwrap();
    assert!(MmapDbphash::open(&corrupt_path).is_err());

    let _ = fs::remove_file(&corrupt_path);
}
