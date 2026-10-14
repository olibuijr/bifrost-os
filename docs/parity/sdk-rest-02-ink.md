# Markup ink model

`bifrost_ui::markup_ink::InkCanvas` captures pen, marker, pencil and eraser pointer gestures as bounded canvas points.
Committed edits support a 100-step undo history; erasing removes whole touched strokes. The shell markup editor still
owns its separate stroke list, and no app or shell editor uses this model yet.

Headless tests cover tool snapshots, clamping, eraser behavior, undo, no-op erases and invalid input. The PencilKit rows
remain partial until an editor uses the shared model and it is checked on Midget.
