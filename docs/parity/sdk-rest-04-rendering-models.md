# Rendering requirement and patch-graph models

The UI crate exposes two pure helpers. `gl_requirements` parses the API family, version and desktop core-profile marker
from a reported GL version string, then checks OpenGL 3.2 Core or GLES 2.0/3.0 floors. `patch_graph` builds a typed patch
DAG, refuses invalid or cyclic connections, and evaluates patches in deterministic dependency order. Both have headless
integration tests.

Neither helper creates a context or draws a composition: apps still cannot query the internal EGL presenter, and the
graph model has no Quartz Composer file format, editor, renderer or plug-in host. Those are the remaining parity gaps.
