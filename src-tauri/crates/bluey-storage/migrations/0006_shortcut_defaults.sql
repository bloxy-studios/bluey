-- UX-001: the first defaults registered ⌘←/→/↑/↓, ⌘⇧↑/↓, ⌘R and ⌘, as global
-- hotkeys, which took those chords away from every other app. Rows that still
-- hold an old default move to the new one; customised rows are left alone.
-- Move is ⌃⌥⌘ + arrows: ⌃⌥ + arrows are Rectangle's and Magnet's half-screen keys.
-- Runs once (schema_migrations), so a user may bind the old chords again later.
UPDATE shortcuts SET accelerator = 'CmdOrCtrl+Ctrl+Alt+ArrowUp'
  WHERE id = 'move_up' AND accelerator = 'CmdOrCtrl+ArrowUp';
UPDATE shortcuts SET accelerator = 'CmdOrCtrl+Ctrl+Alt+ArrowDown'
  WHERE id = 'move_down' AND accelerator = 'CmdOrCtrl+ArrowDown';
UPDATE shortcuts SET accelerator = 'CmdOrCtrl+Ctrl+Alt+ArrowLeft'
  WHERE id = 'move_left' AND accelerator = 'CmdOrCtrl+ArrowLeft';
UPDATE shortcuts SET accelerator = 'CmdOrCtrl+Ctrl+Alt+ArrowRight'
  WHERE id = 'move_right' AND accelerator = 'CmdOrCtrl+ArrowRight';
UPDATE shortcuts SET accelerator = 'CmdOrCtrl+Alt+ArrowUp'
  WHERE id = 'scroll_up' AND accelerator = 'CmdOrCtrl+Shift+ArrowUp';
UPDATE shortcuts SET accelerator = 'CmdOrCtrl+Alt+ArrowDown'
  WHERE id = 'scroll_down' AND accelerator = 'CmdOrCtrl+Shift+ArrowDown';
-- New Chat and Settings keep their keys inside the HUD but are no longer global.
UPDATE shortcuts SET enabled = 0
  WHERE id = 'new_chat' AND accelerator = 'CmdOrCtrl+KeyR';
UPDATE shortcuts SET enabled = 0
  WHERE id = 'open_settings' AND accelerator = 'CmdOrCtrl+Comma';
