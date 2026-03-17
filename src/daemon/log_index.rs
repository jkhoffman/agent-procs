//! Binary sidecar line index for log files.
//!
//! Each log file (e.g. `web.stdout`) has a companion `.idx` file that maps
//! line numbers to byte offsets, enabling random-access reads without
//! scanning the entire log.
//!
//! ## Format
//!
//! ```text
//! Header (16 bytes):
//!   magic:    [u8; 4] = b"LIDX"
//!   version:  u32     = 1       (little-endian)
//!   seq_base: u64     = first sequence number in this file (little-endian)
//!
//! Records (16 bytes each):
//!   byte_offset: u64  = byte position where this line starts (little-endian)
//!   seq:         u64  = global sequence number (little-endian)
//! ```

use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"LIDX";
const VERSION: u32 = 1;
pub const HEADER_SIZE: u64 = 16;
pub const RECORD_SIZE: u64 = 16;

/// Returns the index path for a given log path (appends `.idx`).
pub fn idx_path_for(log_path: &Path) -> PathBuf {
    let mut s = log_path.as_os_str().to_os_string();
    s.push(".idx");
    PathBuf::from(s)
}

/// A single index record mapping a line to its byte offset and sequence number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndexRecord {
    pub byte_offset: u64,
    pub seq: u64,
}

/// Writes index entries to a `.idx` sidecar file.
pub struct IndexWriter {
    writer: BufWriter<File>,
}

impl IndexWriter {
    /// Create a new index file, writing the header with the given `seq_base`.
    pub fn create(path: &Path, seq_base: u64) -> io::Result<Self> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(MAGIC)?;
        writer.write_all(&VERSION.to_le_bytes())?;
        writer.write_all(&seq_base.to_le_bytes())?;
        writer.flush()?;
        Ok(Self { writer })
    }

    /// Append a record to the index.
    pub fn append(&mut self, record: IndexRecord) -> io::Result<()> {
        self.writer.write_all(&record.byte_offset.to_le_bytes())?;
        self.writer.write_all(&record.seq.to_le_bytes())?;
        Ok(())
    }

    /// Flush buffered writes to disk.
    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// Reads index entries from a `.idx` sidecar file.
pub struct IndexReader {
    file: File,
    line_count: usize,
    pub seq_base: u64,
}

impl IndexReader {
    /// Open an existing index file.  Returns `Ok(None)` if the file is too
    /// small, has wrong magic, or wrong version.
    pub fn open(path: &Path) -> io::Result<Option<Self>> {
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();
        if file_len < HEADER_SIZE {
            return Ok(None);
        }

        let mut header = [0u8; 16];
        file.read_exact(&mut header)?;
        if &header[0..4] != MAGIC {
            return Ok(None);
        }
        let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
        if version != VERSION {
            return Ok(None);
        }
        let seq_base = u64::from_le_bytes(header[8..16].try_into().unwrap());

        let data_bytes = file_len - HEADER_SIZE;
        let line_count = (data_bytes / RECORD_SIZE) as usize;

        Ok(Some(Self {
            file,
            line_count,
            seq_base,
        }))
    }

    /// Number of lines indexed in this file.
    pub fn line_count(&self) -> usize {
        self.line_count
    }

    /// Read a single record by 0-based line number.
    pub fn read_record(&mut self, line: usize) -> io::Result<IndexRecord> {
        if line >= self.line_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("line {} out of range (count: {})", line, self.line_count),
            ));
        }
        let offset = HEADER_SIZE + (line as u64) * RECORD_SIZE;
        self.file.seek(SeekFrom::Start(offset))?;
        let mut buf = [0u8; 16];
        self.file.read_exact(&mut buf)?;
        Ok(IndexRecord {
            byte_offset: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            seq: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
        })
    }

    /// Read records `[start..start+count)`, clamped to available lines.
    pub fn read_range(&mut self, start: usize, count: usize) -> io::Result<Vec<IndexRecord>> {
        let end = (start + count).min(self.line_count);
        let actual = end.saturating_sub(start);
        if actual == 0 {
            return Ok(Vec::new());
        }
        let offset = HEADER_SIZE + (start as u64) * RECORD_SIZE;
        self.file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; actual * RECORD_SIZE as usize];
        self.file.read_exact(&mut buf)?;
        let records = buf
            .chunks_exact(RECORD_SIZE as usize)
            .map(|chunk| IndexRecord {
                byte_offset: u64::from_le_bytes(chunk[0..8].try_into().unwrap()),
                seq: u64::from_le_bytes(chunk[8..16].try_into().unwrap()),
            })
            .collect();
        Ok(records)
    }

    /// Compute line count from file metadata without reading content.
    pub fn line_count_from_metadata(path: &Path) -> io::Result<usize> {
        let meta = std::fs::metadata(path)?;
        let len = meta.len();
        if len < HEADER_SIZE {
            return Ok(0);
        }
        Ok(((len - HEADER_SIZE) / RECORD_SIZE) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.idx");

        let mut writer = IndexWriter::create(&path, 100).unwrap();
        writer
            .append(IndexRecord {
                byte_offset: 0,
                seq: 100,
            })
            .unwrap();
        writer
            .append(IndexRecord {
                byte_offset: 42,
                seq: 101,
            })
            .unwrap();
        writer
            .append(IndexRecord {
                byte_offset: 100,
                seq: 102,
            })
            .unwrap();
        writer.flush().unwrap();

        let mut reader = IndexReader::open(&path).unwrap().unwrap();
        assert_eq!(reader.line_count(), 3);
        assert_eq!(reader.seq_base, 100);

        assert_eq!(
            reader.read_record(0).unwrap(),
            IndexRecord {
                byte_offset: 0,
                seq: 100
            }
        );
        assert_eq!(
            reader.read_record(1).unwrap(),
            IndexRecord {
                byte_offset: 42,
                seq: 101
            }
        );
        assert_eq!(
            reader.read_record(2).unwrap(),
            IndexRecord {
                byte_offset: 100,
                seq: 102
            }
        );
    }

    #[test]
    fn test_read_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.idx");

        let mut writer = IndexWriter::create(&path, 0).unwrap();
        for i in 0..10 {
            writer
                .append(IndexRecord {
                    byte_offset: i * 50,
                    seq: i,
                })
                .unwrap();
        }
        writer.flush().unwrap();

        let mut reader = IndexReader::open(&path).unwrap().unwrap();
        assert_eq!(reader.line_count(), 10);

        let range = reader.read_range(3, 4).unwrap();
        assert_eq!(range.len(), 4);
        assert_eq!(range[0].seq, 3);
        assert_eq!(range[3].seq, 6);
    }

    #[test]
    fn test_idx_path_for() {
        assert_eq!(
            idx_path_for(Path::new("/tmp/logs/web.stdout")),
            PathBuf::from("/tmp/logs/web.stdout.idx")
        );
        assert_eq!(
            idx_path_for(Path::new("/tmp/logs/web.stdout.1")),
            PathBuf::from("/tmp/logs/web.stdout.1.idx")
        );
    }

    #[test]
    fn test_empty_file_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.idx");
        std::fs::write(&path, b"tiny").unwrap();
        assert!(IndexReader::open(&path).unwrap().is_none());
    }

    #[test]
    fn test_wrong_magic_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.idx");
        let mut data = vec![0u8; 16];
        data[0..4].copy_from_slice(b"XXXX");
        std::fs::write(&path, &data).unwrap();
        assert!(IndexReader::open(&path).unwrap().is_none());
    }

    #[test]
    fn test_line_count_from_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.idx");

        let mut writer = IndexWriter::create(&path, 0).unwrap();
        writer
            .append(IndexRecord {
                byte_offset: 0,
                seq: 0,
            })
            .unwrap();
        writer
            .append(IndexRecord {
                byte_offset: 10,
                seq: 1,
            })
            .unwrap();
        writer.flush().unwrap();

        assert_eq!(IndexReader::line_count_from_metadata(&path).unwrap(), 2);
    }
}
