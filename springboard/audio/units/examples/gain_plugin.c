/* A plug-in audio unit in C: a gain with one parameter and a hard clip at full scale.
 *
 * Build:  cc -shared -fPIC -O2 -o cgain.so gain_plugin.c
 * Check:  bifrost-unit scan .                       (lists cgain.so with its parameter)
 *         bifrost-unit render ./cgain.so --set 0=2 out.wav
 * Install: copy cgain.so to /usr/lib/bifrost/units/ or ~/.local/share/bifrost/units/ (hosts scan those).
 *
 * The ABI is springboard/audio/units/src/plugin.rs (version 1): the library exports bifrost_unit_entry, which returns
 * a table of functions. The host owns the parameters: render gets each value at the first frame (param_start) and one
 * frame after the last (param_end) and interpolates. render runs on the real-time thread: no allocation, no locks.
 */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    uint32_t kind, accepts_midi, max_channels, param_count;
    char name[64];
    char kind_code[4], sub_kind[4], manufacturer[4];
} CInfo;

typedef struct {
    uint32_t id, unit;
    float min, max, def;
    uint32_t flags, ramp_frames;
    char name[64];
    char group[64];
    char value_strings[192];
} CParam;

typedef struct {
    double sample_rate;
    uint32_t max_frames, input_channels, output_channels;
} CRenderConfig;

typedef struct {
    uint32_t offset;
    uint8_t status, data1, data2, reserved;
} CMidi;

typedef struct {
    uint32_t frames, input_count, output_count, midi_count;
    double sample_time;
    const float *const *inputs;
    float *const *outputs;
    const CMidi *midi;
    uint32_t param_count, reserved;
    const float *param_start;
    const float *param_end;
} CBlock;

typedef struct {
    uint32_t abi_version;
    void (*describe)(CInfo *);
    int32_t (*param)(uint32_t, CParam *);
    void *(*create)(void);
    void (*destroy)(void *);
    int32_t (*initialize)(void *, const CRenderConfig *);
    void (*uninitialize)(void *);
    void (*reset)(void *);
    void (*render)(void *, const CBlock *);
    uint32_t (*latency)(void *);
    uint32_t (*tail)(void *);
} UnitVTable;

static void describe(CInfo *o) {
    memset(o, 0, sizeof *o);
    strcpy(o->name, "C Gain");
    o->kind = 0; /* effect */
    o->max_channels = 2;
    o->param_count = 1;
    memcpy(o->kind_code, "aufx", 4);
    memcpy(o->sub_kind, "cgan", 4);
    memcpy(o->manufacturer, "test", 4);
}

static int32_t param(uint32_t i, CParam *p) {
    if (i != 0) return 1;
    memset(p, 0, sizeof *p);
    p->id = 0;
    p->unit = 5; /* ratio */
    p->min = 0.0f;
    p->max = 4.0f;
    p->def = 1.0f;
    p->ramp_frames = 64;
    strcpy(p->name, "Gain");
    strcpy(p->group, "Main");
    return 0;
}

static void *create(void) { return calloc(1, 1); }
static void destroy(void *p) { free(p); }
static int32_t initialize(void *p, const CRenderConfig *c) { (void)p; (void)c; return 0; }
static void nothing(void *p) { (void)p; }
static uint32_t zero(void *p) { (void)p; return 0; }

static void render(void *p, const CBlock *b) {
    (void)p;
    const float s = b->param_start[0], e = b->param_end[0];
    const uint32_t channels = b->input_count < b->output_count ? b->input_count : b->output_count;
    for (uint32_t c = 0; c < channels; c++) {
        const float *in = b->inputs[c];
        float *out = b->outputs[c];
        for (uint32_t k = 0; k < b->frames; k++) {
            float g = s + (e - s) * (float)k / (float)b->frames;
            float v = in[k] * g;
            out[k] = v > 1.0f ? 1.0f : (v < -1.0f ? -1.0f : v);
        }
    }
}

static const UnitVTable TABLE = {1, describe, param, create, destroy, initialize, nothing, nothing, render, zero, zero};

const UnitVTable *bifrost_unit_entry(void) { return &TABLE; }
