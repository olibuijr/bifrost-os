//! Account desktop shortcuts emitted through one persistent compositor keyboard.
use std::{fs::File,io::{self,Write},os::fd::AsRawFd,sync::{Mutex,OnceLock}};
pub fn shortcut(code:u16) {
    crate::bare::sys::spawn_named("desktop-shortcut", move || {if let Err(e)=send(code){eprintln!("desktop shortcut: {e}");}});
}
fn send(code:u16)->io::Result<()> {
    static KEYBOARD:OnceLock<Mutex<Option<File>>>=OnceLock::new();
    let mut keyboard=KEYBOARD.get_or_init(||Mutex::new(None)).lock().map_err(|_|io::Error::other("keyboard lock"))?;
    if keyboard.is_none() {
        let mut file=std::fs::OpenOptions::new().write(true).open("/dev/uinput")?;
        let fd=file.as_raw_fd();
        if unsafe{super::sys::ioctl(fd,0x40045564,1)}<0{return Err(io::Error::last_os_error());}
        for key in [29u16,30,46,47,45] {if unsafe{super::sys::ioctl(fd,0x40045565,key as i32)}<0{return Err(io::Error::last_os_error());}}
        let mut descriptor=vec![0;1116];let name=b"BifrOSt desktop commands";descriptor[..name.len()].copy_from_slice(name);descriptor[80..82].copy_from_slice(&3u16.to_ne_bytes());
        file.write_all(&descriptor)?;if unsafe{super::sys::ioctl(fd,0x5501)}<0{return Err(io::Error::last_os_error());}
        *keyboard=Some(file);std::thread::sleep(std::time::Duration::from_millis(300));
    }
    let file=keyboard.as_mut().unwrap();
    for (key,value) in [(29,1i32),(code,1),(code,0),(29,0)] {
        let mut event=vec![0;std::mem::size_of::<super::sys::timeval>()];event.extend_from_slice(&1u16.to_ne_bytes());event.extend_from_slice(&key.to_ne_bytes());event.extend_from_slice(&value.to_ne_bytes());file.write_all(&event)?;
        let mut sync=vec![0;std::mem::size_of::<super::sys::timeval>()];sync.extend_from_slice(&[0;8]);file.write_all(&sync)?;
    }Ok(())
}
