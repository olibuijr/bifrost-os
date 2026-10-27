use super::sys::*;
use super::*;

#[test]
fn packets_and_fds_cross_a_socketpair() {
    let (a, b) = socketpair_pair().unwrap();
    let shm = shm_create("test", 4096).unwrap();
    let mut m1 = Map::new(&shm, 4096).unwrap();
    m1.pixels_mut()[3] = 0xdead_beef;
    a.send(&ToShell::Buffers { w: 32, h: 32, generation: 1 }.encode(), &[shm.0]).unwrap();
    a.send(&ToShell::Haptic.encode(), &[]).unwrap();
    let Recv::Packet(p, fds) = b.recv(true).unwrap() else { panic!() };
    assert_eq!(ToShell::decode(&p), Ok(ToShell::Buffers { w: 32, h: 32, generation: 1 }));
    assert_eq!(fds.len(), 1);
    let m2 = Map::new(&fds[0], 4096).unwrap();
    assert_eq!(m2.pixels()[3], 0xdead_beef, "the same memory");
    let Recv::Packet(p, fds) = b.recv(false).unwrap() else { panic!() };
    assert_eq!(ToShell::decode(&p), Ok(ToShell::Haptic));
    assert!(fds.is_empty());
    assert!(matches!(b.recv(false).unwrap(), Recv::Empty));
    drop(a);
    assert!(matches!(b.recv(false).unwrap(), Recv::Closed));
}
