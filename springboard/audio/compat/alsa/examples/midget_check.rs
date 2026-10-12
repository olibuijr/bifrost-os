//! Midget check (needs the running server and the installed plug-in): prints the steps; run the commands on
//! Midget by hand, see docs/audio/08-alsa-compat-plugin.md.

fn main() {
    for step in [
        "speaker-test -D default -c 2 -t sine -l 1",
        "arecord -D default -f S16_LE -r 48000 -c 2 -d 2 /tmp/cap.wav",
        "aplay -D bifrost /usr/share/sounds/alsa/Front_Center.wav",
        "bifrost pc midget open-app ai.storyteller.soundcraft   (while Music plays)",
    ] {
        println!("{step}");
    }
}
