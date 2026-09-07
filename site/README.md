# site/

The project page, served at
[muir.metebalci.com](https://muir.metebalci.com) --- a custom domain over GitHub
Pages, which also answers at
[metebalci.github.io/muir](https://metebalci.github.io/muir).

Hand-written files, no build step and no generator:

    index.html    the front page
    manual.html   the manual: running muir, every flag, the prompt, and
                  how the engines work
    lashup.html   that recording at its own size, which a browser showing
                  the file itself would shrink to the window
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
                  `SimpleTv::png` writes stored deflate blocks and zlib -9
                  puts the same pixels in 2.4K rather than 93K.
    style.css     the stylesheet the three pages share
    .nojekyll     keeps GitHub Pages from running Jekyll over it

`.github/workflows/pages.yml` uploads this directory on every push to `main`
that touches it. The one-time setup is in the repository settings: Pages ->
Build and deployment -> Source: **GitHub Actions**, and Custom domain:
**muir.metebalci.com**, which needs a `CNAME` record for `muir` pointing at
`metebalci.github.io`.

To look at it before pushing, open `index.html` in a browser, or serve the
directory:

    python3 -m http.server -d site 8000
