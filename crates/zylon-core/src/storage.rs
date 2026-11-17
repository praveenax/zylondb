use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{self, ErrorKind, Read, Seek, SeekFrom, Write},
    path::Path,
};

const MAX_RECORD_SIZE: usize = 16 * 1024 * 1024;

pub struct StorageEngine {
    file: File,
    index: HashMap<Vec<u8>, u64>,
}

impl StorageEngine {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        let mut index = HashMap::new();
        let mut offset = 0u64;

        loop {
            file.seek(SeekFrom::Start(offset))?;

            let mut header = [0u8; 8];
            let mut read = 0;

            while read < header.len() {
                let count = file.read(&mut header[read..])?;
                if count == 0 {
                    break;
                }
                read += count;
            }

            if read == 0 {
                break;
            }

            if read < 8 {
                file.set_len(offset)?;
                break;
            }

            let size = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;

            let checksum = u32::from_le_bytes(header[4..8].try_into().unwrap());

            if size < 9 || size > MAX_RECORD_SIZE {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "Invalid record size",
                ));
            }

            let mut payload = vec![0u8; size];

            if let Err(error) = file.read_exact(&mut payload) {
                if error.kind() == ErrorKind::UnexpectedEof {
                    file.set_len(offset)?;
                    break;
                }
                return Err(error);
            }

            if crc32fast::hash(&payload) != checksum {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "Record checksum mismatch",
                ));
            }

            let operation = payload[0];
            let key_len = u32::from_le_bytes(payload[1..5].try_into().unwrap()) as usize;

            let value_len = u32::from_le_bytes(payload[5..9].try_into().unwrap()) as usize;

            if key_len == 0 || key_len.checked_add(value_len) != Some(size - 9) {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "Invalid record layout",
                ));
            }

            let key = payload[9..9 + key_len].to_vec();

            match operation {
                1 => {
                    index.insert(key, offset);
                }
                2 if value_len == 0 => {
                    index.remove(&key);
                }
                _ => {
                    return Err(io::Error::new(ErrorKind::InvalidData, "Invalid operation"));
                }
            }

            offset += 8 + size as u64;
        }

        file.seek(SeekFrom::End(0))?;

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
        let offset = self.append(1, key.as_bytes(), value.as_bytes())?;
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

        if payload[0] != 1 {
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
        self.append(2, key.as_bytes(), &[])?;
        self.index.remove(key.as_bytes());
        Ok(())
    }
}
