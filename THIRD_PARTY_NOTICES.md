# Third-party notices

ether is Apache-2.0 and derived from OpenAI Codex (see LICENSE and NOTICE). In addition, the `ether-tui` crate compiles in the following third-party software, whose licences require these notices to ship with it.

## Mermaid 11.17.2

`crates/tui/src/view_kit/vendor/mermaid.min.js` (npm `mermaid@11.17.2`, unchanged), used to draw diagrams. Licence: MIT.

```
The MIT License (MIT)

Copyright (c) 2014 - 2022 Knut Sveidqvist

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
```

The Mermaid bundle itself contains these libraries; their licence banners are kept inside the minified file:

| Library | Licence |
| --- | --- |
| lodash-es (Copyright OpenJS Foundation and other contributors) | MIT |
| cytoscape | MIT |
| DOMPurify 3.4.12 ((c) Cure53 and other contributors) | Apache-2.0 or MPL-2.0 |

## kitty row/column diacritics

`crates/tui/src/kitty_diacritics.rs` lists the Unicode combining characters kitty uses to number rows and columns of image placeholders, taken from kitty's `rowcolumn-diacritics.txt` (kitty 0.49.2). The list is a selection of code points derived from Unicode's UnicodeData.txt (interface data, not kitty source code); kitty is GPL-3.0.
