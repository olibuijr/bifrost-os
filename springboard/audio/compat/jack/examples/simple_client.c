/* The reference simple_client shape: a sine on both playback ports. Build against our libjack:
 *   cc simple_client.c -o simple_client -ljack -lm
 * (the include is the public header from /usr/include/jack; nothing of the reference implementation is linked). */
#include <jack/jack.h>
#include <math.h>
#include <stdio.h>
#include <unistd.h>

static jack_port_t *out[2];
static float phase = 0.0f;

static int process(jack_nframes_t n, void *arg) {
    (void)arg;
    jack_default_audio_sample_t *l = jack_port_get_buffer(out[0], n);
    jack_default_audio_sample_t *r = jack_port_get_buffer(out[1], n);
    for (jack_nframes_t i = 0; i < n; i++) {
        float s = sinf(phase * 6.2831853f) * 0.2f;
        phase += 440.0f / 48000.0f;
        phase -= floorf(phase);
        l[i] = r[i] = s;
    }
    return 0;
}

int main(void) {
    jack_status_t status;
    jack_client_t *c = jack_client_open("simple_client", JackNullOption, &status);
    if (!c) {
        fprintf(stderr, "cannot open a client, status 0x%x\n", status);
        return 1;
    }
    out[0] = jack_port_register(c, "out_1", JACK_DEFAULT_AUDIO_TYPE, JackPortIsOutput, 0);
    out[1] = jack_port_register(c, "out_2", JACK_DEFAULT_AUDIO_TYPE, JackPortIsOutput, 0);
    jack_set_process_callback(c, process, NULL);
    if (jack_activate(c)) return 1;
    const char **ports = jack_get_ports(c, NULL, NULL, JackPortIsPhysical | JackPortIsInput);
    if (ports) {
        jack_connect(c, jack_port_name(out[0]), ports[0]);
        if (ports[1]) jack_connect(c, jack_port_name(out[1]), ports[1]);
        jack_free(ports);
    }
    sleep(5);
    jack_client_close(c);
    return 0;
}
