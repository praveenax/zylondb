use std::fs;
use zylon_core::StorageEngine;

#[test]
fn persists_updates_and_deletes() {
    let path = std::env::temp_dir().join(format!("zylondb-test-{}.log", std::process::id()));

    let _ = fs::remove_file(&path);

    {
        let mut db = StorageEngine::open(&path).unwrap();

        db.put("user:1", "Alice").unwrap();
        db.put("user:2", "Bob").unwrap();

        assert_eq!(db.get("user:1").unwrap(), Some("Alice".to_string()));

        db.put("user:1", "Alicia").unwrap();
        db.delete("user:2").unwrap();
    }

    // Reopen the database and rebuild its index.
    {
        let mut db = StorageEngine::open(&path).unwrap();

        assert_eq!(db.get("user:1").unwrap(), Some("Alicia".to_string()));

        assert_eq!(db.get("user:2").unwrap(), None);
    }

    fs::remove_file(path).unwrap();
}
