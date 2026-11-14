<!-- Made by Nordan -->
# Third-Party Notices

## WinUtil import table

`crates/windowcraft-config/catalog/winutil-import.json` lists the ids that Chris
Titus Tech's Windows Utility (<https://github.com/ChrisTitusTech/winutil>) writes
into exported configs, next to the matching WindowCraft ids, so that `--config`
can read those files. WinUtil is Copyright (c) 2022 CT Tech Group LLC and
distributed under the MIT License. The full text is in
`crates/windowcraft-config/catalog/WINUTIL-LICENSE.txt`. WindowCraft is not
affiliated with Chris Titus Tech or WinUtil.

Releases up to 1.0.3 shipped catalogs converted from WinUtil's `config/`
directory, with every PowerShell block replaced by a typed native operation.
From 1.1.0 on, the names, descriptions, categories and presets in the catalogs
are written for WindowCraft, and the optional PowerShell profile is WindowCraft's
own instead of a download of the CTT profile.

## WiX Toolset

`setup.exe` contains the WiX Toolset 3.14 standard bootstrapper application,
and the files in `packaging/lang/` are translations of its theme strings.
`packaging/theme.xml` is its `RtfTheme.xml` with one checkbox added, and stays
under the same license. WiX is
Copyright (c) .NET Foundation and contributors and distributed under the
Microsoft Reciprocal License (MS-RL). The full text is in
`packaging/lang/WIX-LICENSE.txt`. Source code: <https://github.com/wixtoolset/wix3>

## Lucide and Feather

The interface icons in `crates/windowcraft-app/src/icons.rs` were drawn for
WindowCraft in the style of Lucide (<https://lucide.dev>). The wrench and eye
icons follow the construction of the Lucide and Feather versions, so their
licenses apply. Lucide is Copyright (c) 2026 Lucide Icons and Contributors under
the ISC License. The Feather icons it includes are Copyright (c) 2013-present
Cole Bemis under the MIT License. Both texts are in
`crates/windowcraft-app/assets/LUCIDE-LICENSE.txt`, and the ISC notice is:

> Permission to use, copy, modify, and/or distribute this software for any
> purpose with or without fee is hereby granted, provided that the above
> copyright notice and this permission notice appear in all copies.
>
> THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
> WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
> MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
> ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
> WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
> ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
> OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

The MIT notice for the Feather icons is:

> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all
> copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.

## LAME

The file converter on the Extensions page encodes MP3 with LAME 4.0, which is
compiled from source into `windowcraft.exe` through the `mp3lame-sys` and
`mp3lame-encoder` crates. LAME is Copyright the LAME project
(<https://lame.sourceforge.io/>) and distributed under the GNU Library General
Public License, version 2 or later; the two crates are under the GNU Lesser
General Public License, version 3. Both texts are in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md). WindowCraft does not modify
LAME. The LAME sources it builds are the ones published in `mp3lame-sys` 0.2.0
on crates.io, and the complete source of WindowCraft is at
<https://github.com/windowcraftdev/WindowCraft>, so `windowcraft.exe` can be
rebuilt with a changed LAME as described under "Build from source" in the
README.

## Symphonia

Sound is decoded by Symphonia 0.5 (<https://github.com/pdeljanov/Symphonia>),
distributed under the Mozilla Public License 2.0. WindowCraft uses it
unmodified. Its source code is published on crates.io and at the address above,
and the license text is in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Tools fetched by extensions

WindowCraft does not ship yt-dlp, ffmpeg or Deno. The Additional formats and
YouTube downloader extensions on the Extensions page fetch them from their own
GitHub releases when you install the extension, and they stay under their own
licenses:

- yt-dlp (<https://github.com/yt-dlp/yt-dlp>), released into the public domain
  under the Unlicense.
- ffmpeg, the builds published by yt-dlp at
  <https://github.com/yt-dlp/FFmpeg-Builds>, the GPL variant, under the GNU
  General Public License. The source code is at <https://ffmpeg.org>.
- Deno (<https://github.com/denoland/deno>), Copyright the Deno authors, under
  the MIT License.

## Rust Crates

Notices and licenses for third-party Rust crates used by WindowCraft are in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
