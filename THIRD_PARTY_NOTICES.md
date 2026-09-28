# Third-party notices

## Storage provider artwork

The storage icons use existing vector artwork from these projects:

- Local folder: [Ant Design Icons](https://github.com/ant-design/ant-design-icons),
  MIT; the two-tone folder is recolored amber.
- Feishu: [Semi Design](https://github.com/DouyinFE/semi-design), MIT;
  the Feishu mark is recolored blue and teal.
- Cloudflare R2: [Lobe Icons](https://github.com/lobehub/lobe-icons), MIT;
  the color Cloudflare mark is unchanged.
- Dropbox: [Simple Icons](https://github.com/simple-icons/simple-icons), CC0;
  the existing repository asset is colored Dropbox blue.

The source URLs, revisions, asset hashes, and full licenses are included in
`public/storage-provider-icons/`. Brand names and logos remain the property
of their owners. The Mallard duck-and-cloud PNG was generated for the selected
Mallard storage icon design.

## Pixel Icon Library

Mallard's pixel-style UI glyphs are adapted from the
[Pixel Icon Library by HackerNoon](https://github.com/hackernoon/pixel-icon-library).
The original icon files are licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

Copyright © 2023 HackerNoon. The glyphs were inlined, recolored through
`currentColor`, and selectively simplified for Mallard's 14–22 px UI sizes.

## Pixelarticons

Selected navigation glyphs are adapted from
[Pixelarticons by Gerrit Halfmann](https://github.com/halfmage/pixelarticons).
The original icon files are licensed under the MIT License.

Copyright © 2019 Gerrit Halfmann. The SVG paths were inlined, recolored
through `currentColor`, and combined where Mallard needs a Git-project glyph.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## ChatGPT mark

Mallard uses a monochrome, crisp-edge rendering of the
[ChatGPT knot mark](https://commons.wikimedia.org/wiki/File:ChatGPT-Logo.svg)
to identify actions that open a ChatGPT conversation. Wikimedia Commons
describes the underlying geometric mark as public domain while noting that it
remains a trademark of OpenAI.

## Departure Mono

Mallard bundles Departure Mono by Helena Zhang, from
[departuremono.com](https://departuremono.com). It is licensed under the SIL
Open Font License 1.1. The complete license text is included at
`public/fonts/DepartureMono-OFL.txt`.

## Compact pixel icon update

The shared desktop and website set follows Joint mock 2. Updated search, file,
clock, copy, lock, link, unlink, chevrons, and close paths come from Pixelarticons
revision `8275e0af7c16aa40c54ea2b90b7af83b1fe4eb4c`. The tool mark uses the wrench
half of `tools.svg`, centered and rotated. The Git folder retains the previous
desktop folder and prompt geometry. The star is from HackerNoon
revision `fa562540941211a9f7e2fb236b9cbd40810d95f1`; its filled state uses the same
outer path with the counter removed. Paths use currentColor. Existing Mallard
content glyphs, provider identities, the duck, and status meanings are retained.

## Session row outline icons

The session metrics and detail actions use unmodified path geometry from
[Tabler Icons v3.46.0](https://github.com/tabler/tabler-icons/tree/v3.46.0/icons/outline):
`hexagons`, `clock`, `activity`, the `info-square` outline, and `chevron-right`.
The SVG paths are inlined in `SessionRowIcon.tsx` and rendered at 14 px with
currentColor and a 2.5-unit stroke (1.75 units for session info).

MIT License

Copyright (c) 2020-2026 Paweł Kuna

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

### Lucide outline icons

The `message-square` and `wrench` paths, plus the dot and stem of `info`, are from
[Lucide 0.468.0](https://github.com/lucide-icons/lucide/tree/0.468.0/icons).
Original geometry is inlined in `SessionRowIcon.tsx`.

The shared `trash`, `terminal`, `git-commit`, and `external-link` icons in
`Icons.tsx` use Lucide 0.468.0 `trash-2`, `square-terminal`,
`git-commit-horizontal`, and `external-link`. They use a 2-unit rounded stroke.
The terminal prompt and frame radius are enlarged for small action buttons;
the commit node is enlarged to match the approved soft outline design.

ISC License

Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2022 as part of Feather (MIT). All other copyright (c) for Lucide are held by Lucide Contributors 2022.

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
