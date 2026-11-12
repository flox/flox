# User-facing message conventions

- **User-visible message syntax, structure, and content:**
  - Use complete sentences. Do not use "I", "we", or "flox"
    as the subject — drop it instead
    ("did not find an environment", not
    "flox did not find an environment").
    Exception: the first sentence of an error may omit the
    subject for easier parsing.
  - Write errors in sentence case, ending with a period.
    Exception: omit the period when system information
    follows on the same line.
  - Strive for one sentence per line.
  - **Line 1:** A generic statement of the problem written
    for a user who has never read the source.
    Example: "Environment already exists at {location}."
  - **Line 2+:** No dead ends — suggest a next step unless
    there truly is none. Put the most actionable information
    last.
  - **Avoid the error when possible:** If you are certain
    there is a single next step, take it automatically and
    print a sentence describing what you did.
    Example: `flox push` without auth should run
    `flox auth login` and print "Auth information not found.
    Redirecting to 'flox auth login'."
  - **Do not surface internal tool output:** Intercept nix
    or git errors and rewrite them at the product level.
- **CLI output conventions:**
  - **Brand names:** "Flox" (capital F), except when
    referring to CLI commands directly (`flox install`).
    "FloxHub" (one word, capital F and H). "Flox Catalog"
    and "Flox Factory" (both words capitalized). Use the
    article where needed: "search the Flox Catalog".
  - **Parameter notation in messages:**
    `<REQUIRED_PARAM>` for required values,
    `[OPTIONAL_PARAM]` for optional values — UPPERCASE,
    angle or square brackets.
    Example: `flox install <PACKAGE> --dir=<PATH>`
  - **Emoji usage:** One emoji per response, two spaces
    after the emoji. Standard map:
    `❌ ERROR:` (errors), `⚠️` (warnings),
    `✨` (creation), `✅` (success/confirmation),
    `🗑️` (removal), `🚀` (new release available),
    `ℹ️` (general notes), `🤖` (announcements unrelated
    to the command), `⬇️`/`⬆️` (pull/push).
  - **Suggest next steps** at the end of success messages
    when there is an obvious path forward. Use "shell
    points" for multiple options:
    ```
    Next:
      $ flox search <PACKAGE>    <- Search for a package
      $ flox install <PACKAGE>   <- Install a package
      $ flox activate            <- Enter the environment
    ```
  - **Line length:** Wrap output at 80 characters.
  - **Voice:** Active voice, US English, plain language.
    Be concise — describe facts as if speaking to a
    junior engineer seeing the terminal for the first time.
  - **Spelling:** Use US English spelling in all text that
    Flox writes: command output, errors, progress messages,
    log messages, help text, and man pages. This applies
    even where Nix spells the same idea the British way:
    print `Realizing`, not `Realising`. Two kinds of text keep
    their original spelling. Show output that Flox relays
    from Nix verbatim, such as the stderr of a failed
    `nix build`. Leave alone the words that the user types
    or a program parses: a Nix flag such as
    `nix-store --realise`, or an API value such as the
    `cancelled` status of a Flox Factory build. The rule
    does not call for renaming identifiers or rewriting
    comments.
  - **Output relayed from Nix:** When Flox shows output from
    Nix, introduce it with a line that Flox writes and that
    names Nix as the source, so that users can tell Nix's
    text, British spellings included, from Flox's.
    `Caught Nix error while locking flake:` is an example.
