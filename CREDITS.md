# Credits and licences

## Wallpapers

Every wallpaper in `springboard/wallpapers/` is a royalty-free photograph from
[Wikimedia Commons](https://commons.wikimedia.org/). They are resized to 1920 px wide and stripped of EXIF
metadata. The installer background (`packaging/archiso/airootfs/usr/share/bifrost/installer/bg.jpg`) is cropped
from `svartifoss_basalt.jpg`.

| File | Photo | Author | Licence | Source |
|---|---|---|---|---|
| `waterfall_idyllic.jpg` | Idyllic landscape with a waterfall | Robert Lukeman | [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/) | [Commons](https://commons.wikimedia.org/wiki/File:Idyllic_landscape_with_a_waterfall_(Unsplash).jpg) |
| `svartifoss_basalt.jpg` | Svartifoss waterfall | Giuseppe Milo | [CC BY 3.0](https://creativecommons.org/licenses/by/3.0/) | [Commons](https://commons.wikimedia.org/wiki/File:Svartifoss_Waterfall_Iceland_Travel_Photography_(210524173).jpeg) |
| `kirkjufell_dusk.jpg` | Kirkjufell, Iceland | Jakub Halun | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) | [Commons](https://commons.wikimedia.org/wiki/File:Kirkjufell,_Iceland,_20240714_1631_0713.jpg) |
| `gullfoss_canyon.jpg` | Gullfoss, Iceland | Pierre Selim | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) | [Commons](https://commons.wikimedia.org/wiki/File:Iceland_-_2017-02-22_-_Gullfoss_-_3684.jpg) |
| `arnarstapi_coast.jpg` | Arnarstapi coast | Chmee2 | [CC BY 3.0](https://creativecommons.org/licenses/by/3.0/) | [Commons](https://commons.wikimedia.org/wiki/File:Landscape_eastern_from_Arnarstapi_(6).jpg) |
| `iceland_falls.jpg` | Iceland waterfall | Oleg Dubyna | [CC BY 3.0](https://creativecommons.org/licenses/by/3.0/) | [Commons](https://commons.wikimedia.org/wiki/File:Iceland_S_Waterfall_(249847799).jpeg) |

The CC BY photos need attribution, which this file provides. The author names are as listed on each Commons page;
if a name is wrong, the Commons page is the reference.

## Fonts

Every font BifrOSt ships is under the [SIL Open Font Licence 1.1](https://openfontlicense.org/). They are fetched
by `springboard/tools/pack.py` from [google/fonts](https://github.com/google/fonts) at commit `bd8f81dd`.

| Font | Use | Authors |
|---|---|---|
| Inter | default system font (`SFUI.ttf`) | The Inter Project Authors |
| JetBrains Mono | default monospaced font (`SFUIMono.ttf`) | The JetBrains Mono Project Authors |
| Noto Sans Symbols 2 | symbols (`Symbols.ttf`) | The Noto Project Authors |
| Geist, Geist Mono | font library | The Geist Project Authors (Vercel) |
| IBM Plex Sans | font library | IBM Corp. |
| Source Sans 3, Source Code Pro | font library | Adobe |
| Fira Code | font library | The Fira Code Project Authors |

## App icons

The built-in app icons (`springboard/res/app-icons/`, packed by `springboard/tools/pack.py`) and the icons of the
Music, News, Podcasts, Voice Memos, Clock and Games apps are drawn by `tools/make-icons.py` for BifrOSt, from
geometric shapes and BifrOSt's own palette. They are covered by the MIT licence like the code.

## Code

MIT licence, see [LICENSE](LICENSE).
