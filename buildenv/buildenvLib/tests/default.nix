let
  buildenvLib = import ../.;
  outdentText = buildenvLib.outdentText;

  outdentTextCases = builtins.listToAttrs (
    builtins.map (
      case:
      let
        input = builtins.readFile ./testData/outdentText/input/${case};
        expectedOutput = builtins.readFile ./testData/outdentText/output/${case};
        testSpec = {
          expr = outdentText input;
          expected = expectedOutput;
        };
      in
      {
        name = "test: ${case}";
        value = testSpec;

      }
    ) (builtins.attrNames (builtins.readDir ./testData/outdentText/input))
  );
in

{
  outdentTextTests = outdentTextCases;
}
