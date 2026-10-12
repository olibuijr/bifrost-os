//! Cross-process 3-node chain (driver in this process, two forked followers) for 10 s; exits non-zero on a missed
//! cycle. Run on the host and on Midget: `cargo run --example chain -p bifrost-audio-shm`.
fn main() {
    match bifrost_audio_shm::selftest::chain_fork(10_000, 1_000) {
        Ok(n) => println!("ok: {n} cycles, no misses"),
        Err(e) => {
            eprintln!("audio.shm: FAILED: {e}");
            std::process::exit(1);
        }
    }
}
