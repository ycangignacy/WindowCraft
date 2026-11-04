# Third-Party Notices

## WinUtil

The application, tweak, feature, AppX, DNS and preset catalogs in
`crates/windowcraft-config/catalog/` derive from the `config/` directory of
Chris Titus Tech's Windows Utility:
<https://github.com/ChrisTitusTech/winutil>

WinUtil is Copyright (c) 2022 CT Tech Group LLC and distributed under the MIT
License. The full text is in `crates/windowcraft-config/catalog/WINUTIL-LICENSE.txt`.

Entries were converted to WindowCraft's schema, and every PowerShell block was
replaced by a typed native operation.

## WiX Toolset

`setup.exe` contains the WiX Toolset 3.14 standard bootstrapper application,
and the files in `packaging/lang/` are translations of its theme strings. WiX is
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

## Rust Crates

Notices and licenses for third-party Rust crates used by WindowCraft are in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
