use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use zylon_core::StorageEngine;

#[test]
fn persists_updates_and_deletes() {
    let path = TempLog::new("persist");

    {
        let mut db = StorageEngine::open(path.path()).unwrap();

        db.put("user:1", "Alice").unwrap();
        db.put("user:2", "Bob").unwrap();

        assert_eq!(db.get("user:1").unwrap(), Some("Alice".to_string()));

        db.put("user:1", "Alicia").unwrap();
        db.delete("user:2").unwrap();
    }

    let mut db = StorageEngine::open(path.path()).unwrap();

    assert_eq!(db.get("user:1").unwrap(), Some("Alicia".to_string()));
    assert_eq!(db.get("user:2").unwrap(), None);
}

#[test]
fn truncates_a_partial_header_and_accepts_new_writes() {
    let path = TempLog::new("partial-header");

    {
        let mut db = StorageEngine::open(path.path()).unwrap();
        db.put("user:1", "Alice").unwrap();
    }

    let committed = file_len(path.path());
    append_bytes(path.path(), &[0x01, 0x02, 0x03]);

    {
        let mut db = StorageEngine::open(path.path()).unwrap();
        assert_eq!(file_len(path.path()), committed);
        assert_eq!(db.get("user:1").unwrap(), Some("Alice".to_string()));
        db.put("user:2", "Bob").unwrap();
    }

    let mut db = StorageEngine::open(path.path()).unwrap();
    assert_eq!(db.get("user:1").unwrap(), Some("Alice".to_string()));
    assert_eq!(db.get("user:2").unwrap(), Some("Bob".to_string()));
}

#[test]
fn truncates_a_partial_record_body() {
    let path = TempLog::new("partial-body");

    {
        let mut db = StorageEngine::open(path.path()).unwrap();
        db.put("user:1", "Alice").unwrap();
        db.delete("user:1").unwrap();
    }

    let committed = file_len(path.path());
    let mut torn = Vec::new();
    torn.extend_from_slice(&64u32.to_le_bytes());
    torn.extend_from_slice(&0u32.to_le_bytes());
    torn.extend_from_slice(&[0u8; 4]);
    append_bytes(path.path(), &torn);

    let mut db = StorageEngine::open(path.path()).unwrap();
    assert_eq!(file_len(path.path()), committed);
    assert_eq!(db.get("user:1").unwrap(), None);
}

#[test]
fn drops_a_final_record_whose_checksum_does_not_match() {
    let path = TempLog::new("bad-tail-checksum");

    {
        let mut db = StorageEngine::open(path.path()).unwrap();
        db.put("user:1", "Alice").unwrap();
        db.put("user:2", "Bob").unwrap();
    }

    let first_end = record_end(path.path(), 0);
    flip_byte(path.path(), file_len(path.path()) - 1);

    let mut db = StorageEngine::open(path.path()).unwrap();
    assert_eq!(file_len(path.path()), first_end);
    assert_eq!(db.get("user:1").unwrap(), Some("Alice".to_string()));
    assert_eq!(db.get("user:2").unwrap(), None);
}

#[test]
fn rejects_checksum_damage_when_later_records_follow() {
    let path = TempLog::new("bad-middle-checksum");

    {
        let mut db = StorageEngine::open(path.path()).unwrap();
        db.put("user:1", "Alice").unwrap();
        db.put("user:2", "Bob").unwrap();
    }

    let original_len = file_len(path.path());
    let first_end = record_end(path.path(), 0);
    flip_byte(path.path(), first_end - 1);

    let error = match StorageEngine::open(path.path()) {
        Ok(_) => panic!("open recovered a checksum mismatch that is not at the tail"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert_eq!(file_len(path.path()), original_len);
}

#[test]
fn rejects_an_illegal_record_length_that_is_not_a_torn_tail() {
    let path = TempLog::new("illegal-length");

    {
        let mut db = StorageEngine::open(path.path()).unwrap();
        db.put("user:1", "Alice").unwrap();
    }

    let original_len = file_len(path.path());
    let mut embedded = Vec::new();
    embedded.extend_from_slice(&1u32.to_le_bytes());
    embedded.extend_from_slice(&0u32.to_le_bytes());
    embedded.extend_from_slice(&[0u8; 8]);
    append_bytes(path.path(), &embedded);

    let error = match StorageEngine::open(path.path()) {
        Ok(_) => panic!("open truncated an illegal record that is not a torn tail"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert_eq!(file_len(path.path()), original_len + embedded.len() as u64);
}

#[test]
fn exclusive_lock_rejects_a_second_open_until_drop() {
    let path = TempLog::new("lock");
    let mut db = StorageEngine::open(path.path()).unwrap();
    db.put("user:1", "Alice").unwrap();

    let error = match StorageEngine::open(path.path()) {
        Ok(_) => panic!("second open succeeded while the database was locked"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ErrorKind::WouldBlock);

    drop(db);

    let mut reopened = StorageEngine::open(path.path()).unwrap();
    assert_eq!(reopened.get("user:1").unwrap(), Some("Alice".to_string()));
}

struct TempLog(PathBuf);

impl TempLog {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("zylondb-{name}-{}.log", std::process::id()));
        let _ = fs::remove_file(&path);
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempLog {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn file_len(path: &Path) -> u64 {
    fs::metadata(path).unwrap().len()
}

fn append_bytes(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

fn flip_byte(path: &Path, index: u64) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    file.seek(SeekFrom::Start(index)).unwrap();
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 0xff;
    file.seek(SeekFrom::Start(index)).unwrap();
    file.write_all(&byte).unwrap();
    file.sync_all().unwrap();
}

fn record_end(path: &Path, offset: u64) -> u64 {
    let mut file = fs::File::open(path).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut header = [0u8; 8];
    file.read_exact(&mut header).unwrap();
    let size = u32::from_le_bytes(header[..4].try_into().unwrap()) as u64;
    offset + 8 + size
}
