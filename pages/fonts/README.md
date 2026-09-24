# `pages/fonts/`

The three faces the pages use, carried here so that reading them asks nothing
of a third party.

| file | face | |
|---|---|---|
| `dela-gothic-one.woff2` | Dela Gothic One | the display face: titles, numerals, the contents |
| `zen-maru-gothic-500.woff2` | Zen Maru Gothic Medium | the voice, which is what these pages call normal |
| `zen-maru-gothic-700.woff2` | Zen Maru Gothic Bold | the voice, emphasized, and the speech bubbles |
| `ibm-plex-mono-400.woff2` | IBM Plex Mono | regular |
| `ibm-plex-mono-500.woff2` | IBM Plex Mono | medium |
| `ibm-plex-mono-600.woff2` | IBM Plex Mono | semibold |

Zen Maru Gothic has no 400 here on purpose. Cold Boot, whose hand these pages
are drawn in, asks Google for 500, 700 and 900 and sets its running text at
the weight a browser then picks for normal, which is 500; `style.css` names
that weight outright rather than leaving it to the matcher.

## Where they came from

The Plex files are Google Fonts' own `latin` subset, fetched on 7 September
2026: every character these pages use is in it.

The other two were cut from the upstream TTFs in
[google/fonts](https://github.com/google/fonts) ---
`ofl/delagothicone/DelaGothicOne-Regular.ttf`,
`ofl/zenmarugothic/ZenMaruGothic-Medium.ttf` and `ZenMaruGothic-Bold.ttf` ---
because they are Japanese families and the whole of one is megabytes. They
were first cut on 15 September 2026 and cut again on 24 September 2026 with
more code points, from upstream files that still gave the first cut byte for
byte. What is kept is the 117 code points the pages actually draw: printable
ASCII, `U+00A0` `U+00A7` `U+00A9` `U+00B5` `U+00B7` `U+00D7` `U+2013`
`U+2014` `U+2019` `U+201C` `U+201D` `U+2026`, and the ten katakana of
ミュア, カチッ, ドン and クックス --- `U+30A2` `U+30AB` `U+30AF` `U+30B9`
`U+30C1` `U+30C3` `U+30C9` `U+30DF` `U+30E5` `U+30F3`. Each file is about
9&nbsp;KB.

    pip install fonttools brotli
    pyftsubset DelaGothicOne-Regular.ttf --flavor=woff2 \
        --unicodes=U+0020-007E,U+00A0,U+00A7,U+00A9,U+00B5,U+00B7,U+00D7,U+2013,U+2014,U+2019,U+201C,U+201D,U+2026,U+30A2,U+30AB,U+30AF,U+30B9,U+30C1,U+30C3,U+30C9,U+30DF,U+30E5,U+30F3 \
        --output-file=dela-gothic-one.woff2

**A page that adds a character not in that list gets a fallback for it**, and
the fallback is a different face. Adding Japanese, or any punctuation past the
list, means cutting the two files again with the new code points in the
`--unicodes` argument.

## License

None of the three is this project's work and none is under its license.

**Dela Gothic One**, by artakana, **Zen Maru Gothic**, by Yoshimichi Ohira,
and **IBM Plex Mono**, by IBM, are all under the **SIL Open Font License,
Version 1.1**, which permits redistribution with or without modification. The
files here are unmodified subsets. The license text travels with each
project:

- Dela Gothic One --- <https://github.com/syakuzen/DelaGothic>
- Zen Maru Gothic --- <https://github.com/googlefonts/zen-marugothic>
- IBM Plex --- <https://github.com/IBM/plex>
