{
  /*
    Outdent a block of text (multi line string) by the minimal common indentation,
    i.e. shift an indented block "to the left",
    so that the content of its least indented line starts at column 1.

    ## Example

    ```
      <line>
        <line>
    ```

    is shifted by 2 spaces to the left:

    ```
    <line>
      <line>
    ```

    ## Type

    ```
    outdentText :: string -> string
    ```
  */
  outdentText =
    # Text to be outdented
    text:

    let
      # Helpers

      # Takes a multi line text and splits it into lines
      #
      # > builtins.split regex str:
      # > Returns a list composed of non matched strings
      # > interleaved with the lists of the extended POSIX regular expression regex matches
      #
      # In the absense of match groups that means we return
      #
      #     [ "<line>" [] "<line>" [] "<line>"... ]
      #
      # Hence the `builtins.filter` to extract the lines only.
      #
      # splitIntoLines :: string -> [string]
      splitIntoLines = text: builtins.filter builtins.isString (builtins.split "\n" text);

      # Takes a list of lines and extracts the leading whitespace from them.
      # The result is used to determine the minimal amount of whitespace
      # common to all lines in the next step.
      #
      # Empty lines have no whitespace and would therefore cause the outdent to be 0 spaces.
      # To avoid that, return `null` for empty lines and carry on.
      #
      # extractLeadingWhitespace :: [string] -> [string | null]
      extractLeadingWhitespace = map (
        line: if line == "" then null else builtins.elemAt (builtins.match "^([ ]*).*" line) 0
      );

      # Finds the shortest string in a list of string.
      # In this case the strings are indentations,
      # hence we find the shortest common indent.
      #
      # We skip over `null` values, which denote empty lines
      # found by the previous step.
      #
      # getMinCommonWhitespace :: [string] -> int | 1000000
      getMinCommonWhitespace =
        let
          # nul value for the fold operation, supposedly larger
          # than any reasonable _minimal_ indentation
          INIT_INDENT = 1000000;
        in
        builtins.foldl' (
          acc: elem:
          if elem == null then
            acc
          else
            let
              len = builtins.stringLength elem;
            in
            if acc < len then acc else len
        ) INIT_INDENT;

      # Remove a given number of characters from the front of each line.
      # In this context we determined `len` to be the minimal common indent,
      # i.e. supposedly spaces only.
      #
      # Empty lines are untouched without error,
      # due to the semantics of `builtins.substring`
      stripCommonPrefix = len: map (line: builtins.substring len (-1) line);

      # Implementation
      # Compose the above functions together
      lines = splitIntoLines text;
      leadingWhiteSpace = extractLeadingWhitespace lines;
      minCommonWhitespace = getMinCommonWhitespace leadingWhiteSpace;
      stripped = stripCommonPrefix minCommonWhitespace lines;
      outdentedText = builtins.concatStringsSep "\n" stripped;
    in
    outdentedText;
}
