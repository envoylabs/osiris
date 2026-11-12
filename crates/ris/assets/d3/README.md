# d3, vendored

`d3.v7.9.0.min.js` is the minified build of d3 7.9.0, as published in
the `d3` package on npm (`dist/d3.min.js`). `ris map --html` inlines it
into the page it writes, so the page opens with no network.

It is committed unchanged. Its SHA-256 is

    f2094bbf6141b359722c4fe454eb6c4b0f0e42cc10cc7af921fc158fceb86539

which `shasum -a 256` over the file reproduces, and which can be
compared with the same file from the published package. To move to
another version, replace the file, this hash and the name in
`src/map.rs`.

d3 is under the ISC licence; `LICENSE` beside this file is its text, as
the licence requires of a copy.
