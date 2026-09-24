# sc-io

The file layer: decoding through symphonia (with encoder delay applied for MP3/AAC), and, as they land, the byte-preserving IFF/tag-block/FLAC/MP3-global-gain writers, the write transaction with backup and journal, the analysis cache and the rekordbox XML and CSV writers.
Invariants: readers never modify files; writers carry every unknown chunk, frame and block verbatim; a write is atomic or it did not happen.
