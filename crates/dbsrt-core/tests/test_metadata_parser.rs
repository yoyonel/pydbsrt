//! Comprehensive unit test suite for dbsrt-core::metadata parser.
//!
//! Validates > 50 patterns including Scene, P2P, VOD, Anime, and real NAS filenames.

use dbsrt_core::metadata::{MediaMetadata, MediaType};
use std::path::Path;

#[test]
fn test_standard_tv_scene_release() {
    let raw = "Silicon.Valley.S02E01.Sand.Hill.Shuffle.1080p.AMZN.WEB-DL.DD.5.1.H.265-SiGMA.mkv";
    let meta = MediaMetadata::parse(raw);

    assert_eq!(meta.clean_title, "Silicon Valley");
    assert_eq!(meta.media_type, MediaType::TvEpisode);
    assert_eq!(meta.season, Some(2));
    assert_eq!(meta.episode, Some(1));
    assert_eq!(meta.resolution.as_deref(), Some("1080p"));
    assert_eq!(meta.source.as_deref(), Some("WEB-DL"));
    assert_eq!(meta.release_group.as_deref(), Some("SiGMA"));
}

#[test]
fn test_standard_movie_scene_release() {
    let raw = "Inception.2010.1080p.BluRay.x264.DTS-WiKi.mkv";
    let meta = MediaMetadata::parse(raw);

    assert_eq!(meta.clean_title, "Inception");
    assert_eq!(meta.media_type, MediaType::Movie);
    assert_eq!(meta.year, Some(2010));
    assert_eq!(meta.season, None);
    assert_eq!(meta.episode, None);
    assert_eq!(meta.resolution.as_deref(), Some("1080p"));
    assert_eq!(meta.source.as_deref(), Some("BluRay"));
    assert_eq!(meta.release_group.as_deref(), Some("WiKi"));
}

#[test]
fn test_anime_standard_pattern() {
    let raw = "[SubsPlease] Sousou no Frieren - 05 (1080p) [ABCD1234].mkv";
    let meta = MediaMetadata::parse(raw);

    assert_eq!(meta.clean_title, "Sousou no Frieren");
    assert_eq!(meta.media_type, MediaType::Anime);
    assert_eq!(meta.episode, Some(5));
    assert_eq!(meta.season, None);
    assert_eq!(meta.resolution.as_deref(), Some("1080p"));
    assert_eq!(meta.release_group.as_deref(), Some("SubsPlease"));
}

#[test]
fn test_anime_erai_raws() {
    let raw = "[Erai-raws] Jujutsu Kaisen 2nd Season - 14 [1080p][HEVC].mkv";
    let meta = MediaMetadata::parse(raw);

    assert_eq!(meta.clean_title, "Jujutsu Kaisen 2nd Season");
    assert_eq!(meta.media_type, MediaType::Anime);
    assert_eq!(meta.episode, Some(14));
    assert_eq!(meta.resolution.as_deref(), Some("1080p"));
    assert_eq!(meta.release_group.as_deref(), Some("Erai-raws"));
}

#[test]
fn test_1x02_syntax() {
    let raw = "Silicon.Valley.1x02.The.Cap.Table.720p.HDTV.x264-IMMERSE.mp4";
    let meta = MediaMetadata::parse(raw);

    assert_eq!(meta.clean_title, "Silicon Valley");
    assert_eq!(meta.media_type, MediaType::TvEpisode);
    assert_eq!(meta.season, Some(1));
    assert_eq!(meta.episode, Some(2));
    assert_eq!(meta.resolution.as_deref(), Some("720p"));
    assert_eq!(meta.source.as_deref(), Some("HDTV"));
    assert_eq!(meta.release_group.as_deref(), Some("IMMERSE"));
}

#[test]
fn test_multi_episode_syntax() {
    let raw = "Game.of.Thrones.S01E01-E02.1080p.BluRay.x264-ROVERS.mkv";
    let meta = MediaMetadata::parse(raw);

    assert_eq!(meta.clean_title, "Game of Thrones");
    assert_eq!(meta.media_type, MediaType::TvEpisode);
    assert_eq!(meta.season, Some(1));
    assert_eq!(meta.episode, Some(1));
    assert_eq!(meta.release_group.as_deref(), Some("ROVERS"));
}

#[test]
fn test_season_pack_complete() {
    let raw = "Kingdom.2019.S01.COMPLETE.720p.NF.WEB-DL.x264-HONE.mkv";
    let meta = MediaMetadata::parse(raw);

    assert_eq!(meta.clean_title, "Kingdom");
    assert_eq!(meta.year, Some(2019));
    assert_eq!(meta.season, Some(1));
    assert_eq!(meta.episode, None);
    assert_eq!(meta.media_type, MediaType::TvEpisode);
    assert_eq!(meta.resolution.as_deref(), Some("720p"));
}

#[test]
fn test_daa_prefix_stripping() {
    let raw = "daa-rick.and.morty.s01e11-1080p.phash";
    let meta = MediaMetadata::parse(raw);

    assert_eq!(meta.clean_title, "rick and morty");
    assert_eq!(meta.season, Some(1));
    assert_eq!(meta.episode, Some(11));
    assert_eq!(meta.resolution.as_deref(), Some("1080p"));
    assert_eq!(meta.media_type, MediaType::TvEpisode);
}

#[test]
fn test_parse_from_path() {
    let path = Path::new("/nas/volume1/tvshow/__PHASH__/The.Mandalorian.S01E01.INTERNAL.1080p.WEB.H264-DEFLATE.phash");
    let meta = MediaMetadata::parse_from_path(path);

    assert_eq!(meta.clean_title, "The Mandalorian");
    assert_eq!(meta.season, Some(1));
    assert_eq!(meta.episode, Some(1));
    assert_eq!(meta.resolution.as_deref(), Some("1080p"));
    assert_eq!(meta.source.as_deref(), Some("WEBRip"));
    assert_eq!(meta.release_group.as_deref(), Some("DEFLATE"));
    assert_eq!(meta.media_type, MediaType::TvEpisode);
}

#[test]
fn test_nas_dataset_batch_parsing() {
    let nas_samples = [
        ("Kingdom.(2019).S01E01.Episode.1.1080p.NF.WEB-DL.DD5.1.x264-MZABI.phash", "Kingdom", Some(1), Some(1)),
        ("Kingdom.2019.S02E01.Episode.1.1080p.NF.WEB-DL.DDP5.1.x264-NTG.phash", "Kingdom", Some(2), Some(1)),
        ("Silicon.Valley.S02E01.Sand.Hill.Shuffle.1080p.AMZN.WEB-DL.DD.5.1.H.265-SiGMA.phash", "Silicon Valley", Some(2), Some(1)),
        ("Silicon.Valley.S06E01.1080p.REPACK.AMZN.WEB-DL.DDP5.1.H.264-NTb.phash", "Silicon Valley", Some(6), Some(1)),
        ("Tales.From.The.Loop.S01E01.1080p.AMZN.WEBRip.DDP5.1.x264-AJP69.phash", "Tales From The Loop", Some(1), Some(1)),
        ("The.Mandalorian.S02E01.1080p.DSNP.WEB-DL.DDP5.1.Atmos.H.264-PHOENiX.phash", "The Mandalorian", Some(2), Some(1)),
        ("The.Mandalorian.S02E02.Chapter.10.The.Passenger.1080p.WEBRip.DDP5.1.Atmos.x264-MZABI.phash", "The Mandalorian", Some(2), Some(2)),
        ("the.mandalorian.s02e05.1080p.web.h264-ggwp.phash", "the mandalorian", Some(2), Some(5)),
        ("Rick.and.Morty.S02E01.1080p.BluRay.x264-YELLOWBiRD.phash", "Rick and Morty", Some(2), Some(1)),
        ("Rick.and.Morty.S03E06.1080p.BluRay.x264-YELLOWBiRD.phash", "Rick and Morty", Some(3), Some(6)),
    ];

    for (filename, expected_title, expected_season, expected_episode) in nas_samples {
        let meta = MediaMetadata::parse(filename);
        assert_eq!(
            meta.clean_title, expected_title,
            "Title mismatch for {}",
            filename
        );
        assert_eq!(
            meta.season, expected_season,
            "Season mismatch for {}",
            filename
        );
        assert_eq!(
            meta.episode, expected_episode,
            "Episode mismatch for {}",
            filename
        );
        assert_eq!(meta.media_type, MediaType::TvEpisode);
    }
}

#[test]
fn test_50_plus_release_naming_patterns() {
    let test_cases = [
        // Movies
        (
            "Blade.Runner.2049.2017.1080p.BluRay.x264-SPARKS",
            "Blade Runner 2049",
            MediaType::Movie,
            Some(2017),
            None,
            None,
            Some("1080p"),
            Some("BluRay"),
        ),
        (
            "Interstellar.2014.2160p.UHD.BluRay.x265-TERMiNAL",
            "Interstellar",
            MediaType::Movie,
            Some(2014),
            None,
            None,
            Some("2160p"),
            Some("BluRay"),
        ),
        (
            "A.Ghost.Story.2017.720p.HDTV.x264-CRiMSON",
            "A Ghost Story",
            MediaType::Movie,
            Some(2017),
            None,
            None,
            Some("720p"),
            Some("HDTV"),
        ),
        (
            "The.Matrix.1999.2160p.4K.BluRay.x265-COCKLiGHT",
            "The Matrix",
            MediaType::Movie,
            Some(1999),
            None,
            None,
            Some("2160p"),
            Some("BluRay"),
        ),
        (
            "Parasite.2019.1080p.WEB-DL.H264-CMRG",
            "Parasite",
            MediaType::Movie,
            Some(2019),
            None,
            None,
            Some("1080p"),
            Some("WEB-DL"),
        ),
        (
            "Oppenheimer.2023.IMAX.1080p.BluRay.x264-FLUX",
            "Oppenheimer",
            MediaType::Movie,
            Some(2023),
            None,
            None,
            Some("1080p"),
            Some("BluRay"),
        ),
        (
            "Dune.Part.Two.2024.2160p.WEB-DL.DDP5.1.Atmos.H.265-FLUX",
            "Dune Part Two",
            MediaType::Movie,
            Some(2024),
            None,
            None,
            Some("2160p"),
            Some("WEB-DL"),
        ),
        (
            "Alien.1979.Director's.Cut.1080p.BluRay.x264-DON",
            "Alien",
            MediaType::Movie,
            Some(1979),
            None,
            None,
            Some("1080p"),
            Some("BluRay"),
        ),
        (
            "Pulp.Fiction.1994.1080p.BluRay.x264-CiNEFiLE",
            "Pulp Fiction",
            MediaType::Movie,
            Some(1994),
            None,
            None,
            Some("1080p"),
            Some("BluRay"),
        ),
        (
            "Spirited.Away.2001.1080p.BluRay.x264-THORA",
            "Spirited Away",
            MediaType::Movie,
            Some(2001),
            None,
            None,
            Some("1080p"),
            Some("BluRay"),
        ),
        // TV Episodes standard SxxExx
        (
            "Breaking.Bad.S05E16.1080p.BluRay.x264-ROVERS",
            "Breaking Bad",
            MediaType::TvEpisode,
            None,
            Some(5),
            Some(16),
            Some("1080p"),
            Some("BluRay"),
        ),
        (
            "Better.Call.Saul.S06E13.1080p.AMZN.WEB-DL.DDP5.1.H.264-NTb",
            "Better Call Saul",
            MediaType::TvEpisode,
            None,
            Some(6),
            Some(13),
            Some("1080p"),
            Some("WEB-DL"),
        ),
        (
            "Succession.S04E10.1080p.WEB-DL.H264-FLUX",
            "Succession",
            MediaType::TvEpisode,
            None,
            Some(4),
            Some(10),
            Some("1080p"),
            Some("WEB-DL"),
        ),
        (
            "The.Bear.S02E06.720p.WEB-DL.x264-FLUX",
            "The Bear",
            MediaType::TvEpisode,
            None,
            Some(2),
            Some(6),
            Some("720p"),
            Some("WEB-DL"),
        ),
        (
            "Severance.S01E09.1080p.ATVP.WEB-DL.DDP5.1.Atmos.H.264-FLUX",
            "Severance",
            MediaType::TvEpisode,
            None,
            Some(1),
            Some(9),
            Some("1080p"),
            Some("WEB-DL"),
        ),
        (
            "Dark.S03E08.1080p.NF.WEB-DL.DDP5.1.x264-NTG",
            "Dark",
            MediaType::TvEpisode,
            None,
            Some(3),
            Some(8),
            Some("1080p"),
            Some("WEB-DL"),
        ),
        (
            "Fargo.S05E10.1080p.WEB.H264-CAKES",
            "Fargo",
            MediaType::TvEpisode,
            None,
            Some(5),
            Some(10),
            Some("1080p"),
            Some("WEBRip"),
        ),
        (
            "True.Detective.S04E06.1080p.WEB-DL.DDP5.1.Atmos.H.264-FLUX",
            "True Detective",
            MediaType::TvEpisode,
            None,
            Some(4),
            Some(6),
            Some("1080p"),
            Some("WEB-DL"),
        ),
        (
            "Chernobyl.S01E05.1080p.BluRay.x264-ROVERS",
            "Chernobyl",
            MediaType::TvEpisode,
            None,
            Some(1),
            Some(5),
            Some("1080p"),
            Some("BluRay"),
        ),
        (
            "Shogun.2024.S01E10.1080p.WEB-DL.DDP5.1.Atmos.H.264-FLUX",
            "Shogun",
            MediaType::TvEpisode,
            Some(2024),
            Some(1),
            Some(10),
            Some("1080p"),
            Some("WEB-DL"),
        ),
        // TV Episodes alternate patterns
        (
            "The.Wire.1x01.The.Target.720p.HDTV.x264-BATV",
            "The Wire",
            MediaType::TvEpisode,
            None,
            Some(1),
            Some(1),
            Some("720p"),
            Some("HDTV"),
        ),
        (
            "The.Sopranos.6x21.Made.in.America.720p.HDTV.x264-BATV",
            "The Sopranos",
            MediaType::TvEpisode,
            None,
            Some(6),
            Some(21),
            Some("720p"),
            Some("HDTV"),
        ),
        (
            "Lost.4x05.The.Constant.720p.HDTV.x264-CTU",
            "Lost",
            MediaType::TvEpisode,
            None,
            Some(4),
            Some(5),
            Some("720p"),
            Some("HDTV"),
        ),
        (
            "Doctor.Who.2005.S10E01.1080p.HDTV.x264-ORGANiC",
            "Doctor Who",
            MediaType::TvEpisode,
            Some(2005),
            Some(10),
            Some(1),
            Some("1080p"),
            Some("HDTV"),
        ),
        (
            "Doctor.Who.2023.S01E01.1080p.WEB-DL.H264-FLUX",
            "Doctor Who",
            MediaType::TvEpisode,
            Some(2023),
            Some(1),
            Some(1),
            Some("1080p"),
            Some("WEB-DL"),
        ),
        // Resolutions
        (
            "Video.Sample.4320p.8K.WEB-DL.x265",
            "Video Sample",
            MediaType::Movie,
            None,
            None,
            None,
            Some("4320p"),
            Some("WEB-DL"),
        ),
        (
            "Video.Sample.2160p.4K.WEB-DL.x265",
            "Video Sample",
            MediaType::Movie,
            None,
            None,
            None,
            Some("2160p"),
            Some("WEB-DL"),
        ),
        (
            "Video.Sample.1080p.WEB-DL.x264",
            "Video Sample",
            MediaType::Movie,
            None,
            None,
            None,
            Some("1080p"),
            Some("WEB-DL"),
        ),
        (
            "Video.Sample.720p.HDTV.x264",
            "Video Sample",
            MediaType::Movie,
            None,
            None,
            None,
            Some("720p"),
            Some("HDTV"),
        ),
        (
            "Video.Sample.480p.DVDRip.xvid",
            "Video Sample",
            MediaType::Movie,
            None,
            None,
            None,
            Some("480p"),
            None,
        ),
        // Sources
        (
            "Sample.Movie.1990.BluRay.x264",
            "Sample Movie",
            MediaType::Movie,
            Some(1990),
            None,
            None,
            None,
            Some("BluRay"),
        ),
        (
            "Sample.Movie.1990.WEB-DL.x264",
            "Sample Movie",
            MediaType::Movie,
            Some(1990),
            None,
            None,
            None,
            Some("WEB-DL"),
        ),
        (
            "Sample.Movie.1990.WEBRip.x264",
            "Sample Movie",
            MediaType::Movie,
            Some(1990),
            None,
            None,
            None,
            Some("WEBRip"),
        ),
        (
            "Sample.Movie.1990.HDTV.x264",
            "Sample Movie",
            MediaType::Movie,
            Some(1990),
            None,
            None,
            None,
            Some("HDTV"),
        ),
        (
            "Sample.Movie.1990.DVDRip.x264",
            "Sample Movie",
            MediaType::Movie,
            Some(1990),
            None,
            None,
            None,
            None,
        ),
        // Anime releases
        (
            "[Erai-raws] Kimetsu no Yaiba - 01 [1080p][Multiple Subtitle].mkv",
            "Kimetsu no Yaiba",
            MediaType::Anime,
            None,
            None,
            Some(1),
            Some("1080p"),
            None,
        ),
        (
            "[Erai-raws] Chainsaw Man - 12 [1080p][HEVC].mkv",
            "Chainsaw Man",
            MediaType::Anime,
            None,
            None,
            Some(12),
            Some("1080p"),
            None,
        ),
        (
            "[SubsPlease] Bocchi the Rock! - 08 (1080p) [B24B3D2B].mkv",
            "Bocchi the Rock!",
            MediaType::Anime,
            None,
            None,
            Some(8),
            Some("1080p"),
            None,
        ),
        (
            "[SubsPlease] Attack on Titan Final Season - 28 (1080p) [E21A4B92].mkv",
            "Attack on Titan Final Season",
            MediaType::Anime,
            None,
            None,
            Some(28),
            Some("1080p"),
            None,
        ),
        (
            "[HorribleSubs] One Piece - 900 [720p].mkv",
            "One Piece",
            MediaType::Anime,
            None,
            None,
            Some(900),
            Some("720p"),
            None,
        ),
        // Special naming cases
        (
            "PROPER-Game.of.Thrones.S08E03.1080p.WEB-DL-GoT",
            "Game of Thrones",
            MediaType::TvEpisode,
            None,
            Some(8),
            Some(3),
            Some("1080p"),
            Some("WEB-DL"),
        ),
        (
            "REPACK.Silicon.Valley.S06E07.1080p.AMZN.WEB-DL-NTb",
            "Silicon Valley",
            MediaType::TvEpisode,
            None,
            Some(6),
            Some(7),
            Some("1080p"),
            Some("WEB-DL"),
        ),
        (
            "The.Lord.of.the.Rings.The.Fellowship.of.the.Ring.2001.Extended.1080p.BluRay.x264-FGT",
            "The Lord of the Rings The Fellowship of the Ring",
            MediaType::Movie,
            Some(2001),
            None,
            None,
            Some("1080p"),
            Some("BluRay"),
        ),
        (
            "2001.A.Space.Odyssey.1968.1080p.BluRay.x264-CtrlHD",
            "2001 A Space Odyssey",
            MediaType::Movie,
            Some(1968),
            None,
            None,
            Some("1080p"),
            Some("BluRay"),
        ),
        (
            "1917.2019.1080p.BluRay.x264-SPARKS",
            "1917",
            MediaType::Movie,
            Some(2019),
            None,
            None,
            Some("1080p"),
            Some("BluRay"),
        ),
        (
            "Blade.Runner.Final.Cut.1982.1080p.BluRay.x264-EbP",
            "Blade Runner Final Cut",
            MediaType::Movie,
            Some(1982),
            None,
            None,
            Some("1080p"),
            Some("BluRay"),
        ),
        (
            "Cowboy.Bebop.The.Movie.2001.1080p.BluRay.x264-THORA",
            "Cowboy Bebop The Movie",
            MediaType::Movie,
            Some(2001),
            None,
            None,
            Some("1080p"),
            Some("BluRay"),
        ),
        (
            "South.Park.S26E01.1080p.WEB-DL.x264-FLUX",
            "South Park",
            MediaType::TvEpisode,
            None,
            Some(26),
            Some(1),
            Some("1080p"),
            Some("WEB-DL"),
        ),
        (
            "Rick.and.Morty.S07E10.1080p.WEB-DL.x264-FLUX",
            "Rick and Morty",
            MediaType::TvEpisode,
            None,
            Some(7),
            Some(10),
            Some("1080p"),
            Some("WEB-DL"),
        ),
        (
            "House.of.the.Dragon.S02E08.1080p.WEB-DL.x264-FLUX",
            "House of the Dragon",
            MediaType::TvEpisode,
            None,
            Some(2),
            Some(8),
            Some("1080p"),
            Some("WEB-DL"),
        ),
    ];

    assert!(
        test_cases.len() >= 50,
        "Total test cases: {}",
        test_cases.len()
    );

    for (raw, exp_title, exp_type, exp_year, exp_season, exp_episode, exp_res, exp_src) in
        test_cases
    {
        let meta = MediaMetadata::parse(raw);
        assert_eq!(meta.clean_title, exp_title, "Title failed for '{}'", raw);
        assert_eq!(meta.media_type, exp_type, "MediaType failed for '{}'", raw);
        if exp_year.is_some() {
            assert_eq!(meta.year, exp_year, "Year failed for '{}'", raw);
        }
        if exp_season.is_some() {
            assert_eq!(meta.season, exp_season, "Season failed for '{}'", raw);
        }
        if exp_episode.is_some() {
            assert_eq!(meta.episode, exp_episode, "Episode failed for '{}'", raw);
        }
        if exp_res.is_some() {
            assert_eq!(
                meta.resolution.as_deref(),
                exp_res,
                "Resolution failed for '{}'",
                raw
            );
        }
        if exp_src.is_some() {
            assert_eq!(
                meta.source.as_deref(),
                exp_src,
                "Source failed for '{}'",
                raw
            );
        }
    }
}

#[test]
fn test_percent_encoded_names_parse_like_decoded_ones() {
    // (input, clean_title, season, episode): URL-encoded names (FTP/HTTP listings) must not
    // hide SxxEyy; sequences that would change the path or inject control chars stay encoded.
    let cases = [
        (
            "Show%20Name%20S01E02%201080p.mkv",
            "Show Name",
            Some(1),
            Some(2),
        ),
        (
            "Show%20Name%20S01E02%20-%20Pilot.phash",
            "Show Name",
            Some(1),
            Some(2),
        ),
        (
            "Show.Name.S03E04.100%.Real.1080p.mkv",
            "Show Name",
            Some(3),
            Some(4),
        ),
        ("Show%2FName%20S01E02.mkv", "Show%2FName", Some(1), Some(2)),
        ("Show%00Name%20S01E02.mkv", "Show%00Name", Some(1), Some(2)),
        ("Show%ZZName%20S01E02.mkv", "Show%ZZName", Some(1), Some(2)),
    ];
    for (raw, title, season, episode) in cases {
        let meta = MediaMetadata::parse(raw);
        assert_eq!(
            (meta.clean_title.as_str(), meta.season, meta.episode),
            (title, season, episode),
            "{raw}"
        );
        assert_eq!(meta.raw_title, raw, "raw_title keeps the original input");
    }
}
