# Desktop controls

Shell 0.4.28 uses network and power icons in the desktop menu bar. CPU and RAM
readings appear by default beside them. CPU uses successive /proc/stat counters
(excluding duplicate guest counters); RAM uses MemTotal minus MemAvailable.
The sampler runs on a worker once a second. Dropdowns show processor count,
load averages, uptime, available/installed/cache memory and swap.

The File menu can maximize or restore the active SDK/native window. XDG
configure state updates the SDK window model, including compositor changes.
The green SDK control toggles maximize/restore and shows its state.

Right-click a dock icon to open/show, quit or pin/unpin it. Drag an icon onto
another to reorder pins. Right-click the dock or use BifrOSt → Dock Settings
for Bottom/Left/Right placement, smaller/larger icons and autohide. Hidden docks
reveal at the chosen screen edge, then hide after the pointer leaves. Choices
persist in /var/lib/springboard/desktop-dock.ini using app identities. Running
unpinned apps remain visible. Applications lists shell apps alongside Linux
apps, so removing a pin does not remove the application.

BifrOSt Browser is the HTTP/HTTPS default. New setup installs it from the PC
App Store; normal system packages carry the launcher/association, while browser
updates remain independent. Files 1.0.5 adopts the SDK context-menu API.

CLI: bifrost pc midget context X Y, hover X Y, open-app ID, open-url URL, open-folder ID PATH,
show-in-files PATH. Store apps open by their installed slot (0.4.48); a window closes when its app
exits (0.4.49). State
includes dock choices/visibility, current menu and CPU/memory percentages.
