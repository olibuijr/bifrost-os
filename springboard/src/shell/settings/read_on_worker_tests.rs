use super::read_on_worker;

#[test]
fn a_read_runs_off_the_caller_and_delivers_its_value() {
    let caller = std::thread::current().id();
    let rx = read_on_worker("test-read", move || (7usize, std::thread::current().id()));
    let (value, worker) = rx.recv().expect("the worker sends its result");
    assert_eq!(value, 7);
    assert_ne!(worker, caller, "the read must not run on the calling thread");
}
