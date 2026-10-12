//! `NullDriver`: devices on a manual clock for deterministic host tests (owner: task 05). `advance(ns)` moves every
//! device's hardware pointer by the frames of that time at its rate times (1 + drift_ppm / 1e6).
