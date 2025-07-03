#![cfg(test)]

// These tests involve async files
use std::fs;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::thread;
use std::time::Duration;

use super::Promise;

#[test]
pub fn test_new_creates_file() {
    let path = "/tmp/promise-new".to_string();
    let _ = fs::remove_file(&path); // Clean up before
    let path_for_promise = path.clone();
    let result = Promise::new(move || {
        File::create(&path_for_promise).map(|_| ()).map_err(|e| e)
    }).wait();
    assert!(result.is_ok());
    thread::sleep(Duration::from_millis(100));
    assert!(File::open(&path).is_ok());
    fs::remove_file(&path).unwrap();
}

#[test]
pub fn test_then_creates_file_in_dir() {
    let dir = "/tmp/promise-then".to_string();
    let file = format!("{}/file", dir);
    let _ = fs::remove_dir_all(&dir);
    let file_clone = file.clone();
    let dir_for_promise = dir.clone();
    let result = Promise::new(move || {
        fs::create_dir(dir_for_promise.clone()).map(|_| ()).map_err(|e| e)
    })
    .then(move |res| match res {
        Ok(_) => File::create(file_clone.clone()).map(|_| ()).map_err(|e| e),
        Err(_) => Err(io::Error::new(io::ErrorKind::Other, "Couldn't make dir!")),
    })
    .wait();
    assert!(result.is_ok());
    thread::sleep(Duration::from_millis(100));
    assert!(File::open(&file).is_ok());
    fs::remove_dir_all(&dir).unwrap();
}

#[derive(Debug, PartialEq)]
enum TestErrType {
    CreateDir,
    CreateFile,
    WriteFile,
}

#[test]
pub fn test_map_chain_and_map_err() {
    let dir = "/tmp/promise-then-ok/".to_string();
    let file = format!("{}file", dir);
    let _ = fs::remove_dir_all(&dir);
    let file_clone = file.clone();
    let dir_for_promise = dir.clone();
    let result = Promise::new(move || match fs::create_dir(dir_for_promise.clone()) {
        Ok(_) => Ok(()),
        Err(_) => Err(TestErrType::CreateDir),
    })
    .map(move |_| match File::create(file_clone.clone()) {
        Ok(file) => Ok(file),
        Err(_) => Err(TestErrType::CreateFile),
    })
    .map(|mut file| match file.write_all(b"Hello world!") {
        Ok(_) => Ok(()),
        Err(_) => Err(TestErrType::WriteFile),
    })
    .map_err(|e| {
        println!("File process errored at {:?}", e);
        Err(())
    })
    .wait();
    assert!(result.is_ok());
    thread::sleep(Duration::from_millis(100));
    let mut s = String::new();
    let maybe_file = File::open(&file);
    assert!(maybe_file.is_ok());
    let mut file = maybe_file.unwrap();
    assert!(file.read_to_string(&mut s).is_ok());
    assert_eq!(s, "Hello world!");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
pub fn test_promise_all() {
    let mut p: Vec<Promise<u32, u32>> = Vec::new();
    for x in 0..10 {
        p.push(
            Promise::new(move || {
                thread::sleep(Duration::from_millis(10));
                Ok(x)
            })
            .then(move |res| match res {
                Ok(t) => {
                    assert_eq!(t, x);
                    Ok(t)
                }
                Err(e) => Err(e),
            }),
        );
    }
    let result = Promise::all(p).wait().unwrap().unwrap();
    assert_eq!(result.len(), 10);
    for (i, val) in result.into_iter().enumerate() {
        assert_eq!(val, i as u32);
    }
}

#[test]
pub fn test_promise_error_propagation() {
    let p = Promise::new(|| Err::<u32, &'static str>("fail"))
        .then(|res| match res {
            Ok(_) => Ok(42),
            Err(e) => Err(e),
        });
    let result = p.wait();
    assert_eq!(result, Ok(Err("fail")));
}

#[test]
pub fn test_promise_double_resolution() {
    let p = Promise::new(|| Ok::<i32, ()>(1));
    let first = p.wait();
    let second = first.clone();
    assert_eq!(first, Ok(Ok(1)));
    assert_eq!(second, Ok(Ok(1)));
}

#[test]
pub fn test_promise_drop_behavior() {
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    let dropped = Arc::new(AtomicBool::new(false));
    {
        let dropped = dropped.clone();
        struct Dropper(Arc<AtomicBool>);
        impl Drop for Dropper {
            fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); }
        }
        let d = Dropper(dropped);
        let _ = Promise::new(move || Ok::<Dropper, ()>(d)).wait();
    }
    thread::sleep(Duration::from_millis(10));
    assert!(dropped.load(Ordering::SeqCst));
}
