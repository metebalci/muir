# site/

The front page and the recording, served at
[muir.metebalci.com](https://muir.metebalci.com) --- a custom domain over GitHub
Pages, which also answers at
[metebalci.github.io/muir](https://metebalci.github.io/muir).

The manual is not here. It is Markdown in the repository, `docs/manual.md`
and its companions, read on GitHub where the reader already has the tree
open; the pages here link to it.

Hand-written files, no build step and no generator:

    index.html    the front page, drawn in the hand of Cold Boot --- the
                  manga-style zine about the same machine by the same
                  author. The zine is the story; this is the machine
    lashup.html   the recording alone at its own size, black behind it and
                  nothing else: a browser showing the file itself would
                  shrink it to the window, and 1-bit text scaled is mush
    lashup.gif    the acceptance test recorded as it ran: 1538 x 985, the
                  size the machines drew it and the size to read it at.
                  Made by
                  `cargo test --release --test cc_test_machine -- --ignored`
    lashup-small.gif
                  the same recording at 1240 x 794 for the front page,
                  which shows it 620 wide: a quarter of the bytes and
                  still sharp on a dense screen. Cut with
                  `magick lashup.gif -coalesce -resize 1240x -layers optimize -colors 16 lashup-small.gif`
    system-100.png
                  the screenshot in step 4 of the install: System 100 at its
                  Lisp Listener, 768 x 963, which is the machine's own frame
                  buffer and the size the page shows it at. Made by starting
                  `target/release/muir` as step 4 says, waiting for the boot,
                  and typing `ss` at the prompt; then recompressed, since
                  `Tv::png` writes stored deflate blocks and zlib -9
                  puts the same pixels in 2.4K rather than 93K.
    style.css     the stylesheet the two pages share: ink and paper with
                  one spot color, the size tokens every `font-size` comes
                  from, and no dark mode --- a spot color printed on black
                  is a different object
    fonts/        Dela Gothic One for the display, Zen Maru Gothic for the
                  voice and IBM Plex Mono for the machine's own, served from
                  here rather than from Google so that a visitor need ask
                  nobody else for a page to be readable. `fonts/README.md`
                  says where each came from, which code points the two
                  Japanese families were cut down to, and under what license
    .nojekyll     keeps GitHub Pages from running Jekyll over it

The drawings of CADR are inline SVG in `index.html`, defined once at the top
of the file and placed with `<use>`: the body, the faces and the waving arm
are Cold Boot's own parts, and the magnifier arm and the tired face are drawn
for this page in the same hand. The two diagrams --- the machine, and one
microinstruction through three engines --- are inline SVG too, drawn in their
own viewBox units so that they scale with the box they sit in. The machine
diagram is the site's alone: the manual's own section about the machine
links to it here.

`.github/workflows/pages.yml` uploads this directory on every push to `main`
that touches it. The one-time setup is in the repository settings: Pages ->
Build and deployment -> Source: **GitHub Actions**, and Custom domain:
**muir.metebalci.com**, which needs a `CNAME` record for `muir` pointing at
`metebalci.github.io`.

To look at it before pushing, open `index.html` in a browser, or serve the
directory:

    python3 -m http.server -d site 8000
