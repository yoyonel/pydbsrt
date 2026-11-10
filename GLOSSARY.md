# pydbsrt Media & Alignment Engine

Core domain terminology for media discovery, perceptual hashing, video container identity, and subtitle synchronization.

## Language

### Media Sources & Storage

**MediaSource**:
A configured storage root or network share where candidate video containers are discovered.
_Avoid_: Library, directory, mountpoint, folder

**StorageKind**:
The physical or network protocol backing a MediaSource (`Local`, `CifsMount`, `RemoteStream`).
_Avoid_: Drive type, storage type

### Media Assets & Containers

**MediaAsset**:
A physical video container file discovered within a MediaSource, uniquely identified by its container hash and file size.
_Avoid_: Video file, movie, file, release, media item

**MasterAsset**:
A full-length, untruncated MediaAsset representing a complete creative work (film, TV episode, documentary).
_Avoid_: Full video, complete movie, master release

**DerivedCut**:
A temporal video excerpt extracted from a MasterAsset, possessing an explicit parent asset reference and a bounded sample time range.
_Avoid_: Snippet, cut, short clip, slice, sample

**SyntheticQuery**:
A synthetic or benchmark video sample generated to stress-test trajectory matching and consensus algorithms.
_Avoid_: Query cut, test file, mock video

**MovieHash**:
A 64-bit unsigned checksum computed on a video file's first 64 KiB, last 64 KiB, and total file size, serialized as 16 hexadecimal characters.
_Avoid_: File hash, container hash, video checksum, container_bin_hash

### Perceptual Fingerprints & Integrity

**PHashArtifact**:
A contiguous sequence of 64-bit DCT-II perceptual hashes derived from a decoded MediaAsset, strictly bound to that asset's identity.
_Avoid_: .phash file, fingerprint stream, visual digest, phash dump

**MediaHash**:
A 32-character hexadecimal SHA-256 digest computed over the raw big-endian bytes of a PHashArtifact.
_Avoid_: phash_digest, video_digest, stream_hash

**CompletenessGate**:
An algorithmic invariant verifying that a PHashArtifact's frame count matches its MediaAsset's probed container duration within a 2% tolerance.
_Avoid_: Duration check, frame count validation, size guard

### Subtitles & Couplings

**SubtitleTrack**:
A subtitle stream embedded inside a MediaAsset container or present alongside it as an external sidecar file.
_Avoid_: Sub file, srt file, text stream

**GroundTruthCoupling**:
A certified tripartite relational coupling linking a MediaAsset, its validated PHashArtifact, and one or more SubtitleTrack instances.
_Avoid_: Media coupling, srt link, certification record

**Retargeting**:
The automatic recalibration of a SubtitleTrack's timeline to align with a target MediaAsset using cross-video pHash trajectory consensus.
_Avoid_: Resync, visual sync, shifting
