use std::{
    collections::HashMap,
    fs::{File, OpenOptions, TryLockError},
    io::{self, ErrorKind, Read, Seek, SeekFrom, Write},
    path::Path,
};

const MAX_RECORD_SIZE: usize = 16 * 1024 * 1024;
const OP_PUT: u8 = 1;
const OP_DELETE: u8 = 2;

pub struct StorageEngine {
    file: File,
    index: HashMap<Vec<u8>, u64>,
}

impl StorageEngine {
    /// Open a database log and rebuild its index.
    ///
    /// The open takes an exclusive lock on the file and holds it until this
    /// value is dropped. A second open fails while that lock is held. If the
    /// process crashes, the operating system releases the lock.
    ///
    /// A crash can leave a partial record, or a final record whose checksum
    /// does not match, at the end of the file. That torn suffix is truncated
    /// and synced. A damaged record with bytes after it is left unchanged and
    /// returned as an error, so recovery does not discard later records.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        lock_exclusive(&file)?;
        let index = recover(&mut file)?;

        Ok(Self { file, index })
    }

    fn append(&mut self, operation: u8, key: &[u8], value: &[u8]) -> io::Result<u64> {
        if key.is_empty() || key.len() > u32::MAX as usize || value.len() > u32::MAX as usize {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Invalid key or value size",
            ));
        }

        let size = 9usize
            .checked_add(key.len())
            .and_then(|n| n.checked_add(value.len()))
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Record too large"))?;

        if size > MAX_RECORD_SIZE {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Record exceeds size limit",
            ));
        }

        let mut payload = Vec::with_capacity(size);
        payload.push(operation);
        payload.extend_from_slice(&(key.len() as u32).to_le_bytes());
        payload.extend_from_slice(&(value.len() as u32).to_le_bytes());
        payload.extend_from_slice(key);
        payload.extend_from_slice(value);

        let checksum = crc32fast::hash(&payload);
        let offset = self.file.seek(SeekFrom::End(0))?;

        self.file.write_all(&(size as u32).to_le_bytes())?;
        self.file.write_all(&checksum.to_le_bytes())?;
        self.file.write_all(&payload)?;
        self.file.sync_all()?;

        Ok(offset)
    }

    pub fn put(&mut self, key: &str, value: &str) -> io::Result<()> {
        let offset = self.append(OP_PUT, key.as_bytes(), value.as_bytes())?;
        self.index.insert(key.as_bytes().to_vec(), offset);
        Ok(())
    }

    pub fn get(&mut self, key: &str) -> io::Result<Option<String>> {
        let Some(&offset) = self.index.get(key.as_bytes()) else {
            return Ok(None);
        };

        self.file.seek(SeekFrom::Start(offset))?;

        let mut header = [0u8; 8];
        self.file.read_exact(&mut header)?;

        let size = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;

        if size < 9 || size > MAX_RECORD_SIZE {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "Invalid indexed record size",
            ));
        }

        let checksum = u32::from_le_bytes(header[4..8].try_into().unwrap());

        let mut payload = vec![0u8; size];
        self.file.read_exact(&mut payload)?;

        if crc32fast::hash(&payload) != checksum {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "Indexed record checksum mismatch",
            ));
        }

        if payload[0] != OP_PUT {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "Index points to non-value record",
            ));
        }

        let key_len = u32::from_le_bytes(payload[1..5].try_into().unwrap()) as usize;

        let value_len = u32::from_le_bytes(payload[5..9].try_into().unwrap()) as usize;

        if key_len.checked_add(value_len) != Some(size - 9)
            || payload.get(9..9 + key_len) != Some(key.as_bytes())
        {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "Indexed record layout mismatch",
            ));
        }

        let value = String::from_utf8(payload[9 + key_len..].to_vec())
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;

        Ok(Some(value))
    }

    pub fn delete(&mut self, key: &str) -> io::Result<()> {
        self.append(OP_DELETE, key.as_bytes(), &[])?;
        self.index.remove(key.as_bytes());
        Ok(())
    }
}

fn lock_exclusive(file: &File) -> io::Result<()> {
    match file.try_lock() {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(io::Error::new(
            ErrorKind::WouldBlock,
            "database file is locked by another process",
        )),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

fn recover(file: &mut File) -> io::Result<HashMap<Vec<u8>, u64>> {
    let file_len = file.seek(SeekFrom::End(0))?;
    let mut index = HashMap::new();
    let mut offset = 0u64;

    while offset < file_len {
        let remaining = file_len - offset;
        if remaining < 8 {
            discard_torn_tail(file, offset)?;
            break;
        }

        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0u8; 8];
        if let Err(error) = file.read_exact(&mut header) {
            if error.kind() == ErrorKind::UnexpectedEof {
                discard_torn_tail(file, offset)?;
                break;
            }
            return Err(error);
        }

        let size = u32::from_le_bytes(header[..4].try_into().unwrap()) as u64;
        let checksum = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let Some(record_end) = offset.checked_add(8).and_then(|end| end.checked_add(size)) else {
            discard_torn_tail(file, offset)?;
            break;
        };

        // A header written by a crash can claim a length that runs past EOF.
        // An illegal length that still sits inside the file is corruption:
        // later records may follow it, so the tail stays on disk.
        if size < 9 || size > MAX_RECORD_SIZE as u64 {
            if record_end > file_len {
                discard_torn_tail(file, offset)?;
                break;
            }
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "Invalid record size",
            ));
        }

        if record_end > file_len {
            discard_torn_tail(file, offset)?;
            break;
        }

        let mut payload = vec![0u8; size as usize];
        if let Err(error) = file.read_exact(&mut payload) {
            if error.kind() == ErrorKind::UnexpectedEof {
                discard_torn_tail(file, offset)?;
                break;
            }
            return Err(error);
        }

        if crc32fast::hash(&payload) != checksum {
            if record_end == file_len {
                discard_torn_tail(file, offset)?;
                break;
            }
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "Record checksum mismatch",
            ));
        }

        apply_record(&mut index, &payload, offset)?;
        offset = record_end;
    }

    file.seek(SeekFrom::End(0))?;
    Ok(index)
}

fn discard_torn_tail(file: &File, len: u64) -> io::Result<()> {
    file.set_len(len)?;
    file.sync_all()
}

fn apply_record(index: &mut HashMap<Vec<u8>, u64>, payload: &[u8], offset: u64) -> io::Result<()> {
    if payload.len() < 9 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "Invalid record layout",
        ));
    }

    let operation = payload[0];
    let key_len = u32::from_le_bytes(payload[1..5].try_into().unwrap()) as usize;
    let value_len = u32::from_le_bytes(payload[5..9].try_into().unwrap()) as usize;

    if key_len == 0 || key_len.checked_add(value_len) != Some(payload.len() - 9) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "Invalid record layout",
        ));
    }

    let key = payload[9..9 + key_len].to_vec();

    match operation {
        OP_PUT => {
            index.insert(key, offset);
        }
        OP_DELETE if value_len == 0 => {
            index.remove(&key);
        }
        _ => {
            return Err(io::Error::new(ErrorKind::InvalidData, "Invalid operation"));
        }
    }

    Ok(())
}
