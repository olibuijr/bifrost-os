# Folder Actions

Code: `springboard/src/shell/folder_actions.rs`. The shell tick drives it.

An app intent attached to a folder runs once for each file that appears in that folder.

## Model

- `Registry` holds the attachments: a folder and the intent to run.
- `Watcher` owns one `Registry` and rescans each attached folder once a second (`poll`). The folder's listing goes to
  `Registry::on_listing`, and the answer lists the runs to start.
- Each run calls the attached intent (`shell::intents`) with the new file's name as the `file` parameter.
- Attaching records the folder's current contents first, so the files already there do not run the action.
