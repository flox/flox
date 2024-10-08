{
  description = "Super Resolver";

  inputs = {
    # Empty inputs to be overridden
    a.follows = "/";
    b.follows = "/";
    c.follows = "/";
    d.follows = "/";
    e.follows = "/";
    f.follows = "/";
    g.follows = "/";
    h.follows = "/";
    i.follows = "/";
    j.follows = "/";
    k.follows = "/";
    l.follows = "/";
    m.follows = "/";
    n.follows = "/";
    o.follows = "/";
    p.follows = "/";
    q.follows = "/";
    r.follows = "/";
    s.follows = "/";
    t.follows = "/";
  };

  outputs = reflected: rec {
    resolve =
      {
        # List of string input names, to control which inputs we actually scan
        inputs,
        # List of default prefixnames we scan
        defaultPrefixes,
        # The current system to resolve for
        system,
        # A list of strings comprising an attrpath to be applied to reach the potential installable
        # This may be missing a system, a prefix, or both
        key,
        # Optional function used to filter out fields and find descriptions
        processor ? null,
      }:
      let
        # Copied from nixpkgs
        attrByPath =
          attrPath: default: set:
          let
            attr = builtins.head attrPath;
          in
          if attrPath == [ ] then
            set
          else if set ? ${attr} then
            attrByPath (builtins.tail attrPath) default set.${attr}
          else
            default;
        # Copied from nixpkgs
        hasAttrByPath =
          attrPath: e:
          let
            attr = builtins.head attrPath;
          in
          if attrPath == [ ] then
            true
          else if e ? ${attr} then
            hasAttrByPath (builtins.tail attrPath) e.${attr}
          else
            false;

        # Generate a list of prefix and key pairs for the given key
        prefixKeyPairs =
          (map (defaultPrefix: {
            prefix = defaultPrefix;
            inherit key;
          }) defaultPrefixes)
          ++ (
            if builtins.length key >= 2 then
              [
                {
                  prefix = builtins.head key;
                  key = builtins.tail key;
                }
              ]
            else
              [ ]
          );

        # Get the prefix from an input by name, accounting for systemization if present
        selectPrefix =
          input: prefixName:
          if input.${prefixName} ? "${system}" then input.${prefixName}.${system} else input.${prefixName};

        # Create a list of all matching keys (remember keys are a list of strings too).
        keysForInputAndPrefix =
          input: pair:
          let
            prefix = selectPrefix input pair.prefix;
          in
          # If no key, all available keys
          if builtins.length pair.key == 0 then
            { keys = map (x: [ x ]) (builtins.attrNames prefix); }
          # If provided key matches on systemized prefix, just the key
          else if hasAttrByPath pair.key prefix then
            { keys = [ pair.key ]; }
          # If provided key matches on prefix without system, use the first component of the key as a system
          else if
            systemIfPresent input pair.prefix != null && hasAttrByPath pair.key input.${pair.prefix}
          then
            {
              keys =
                if
                  builtins.length pair.key >= 2
                # If key is long enough to include a tail, then the tail can be the key
                then
                  [ (builtins.tail pair.key) ]
                # If key only includes the system, then all available keys under than system
                else
                  map (x: [ x ]) (builtins.attrNames input.${pair.prefix}.${builtins.head pair.key});
              system = builtins.head pair.key;
            }
          # Else no key matches
          else
            { keys = [ ]; };

        # The current system to resolve for if a system follows the prefix name on the input
        systemIfPresent =
          input: prefixName: if input.${prefixName} ? "${system}" then "${system}" else null;

        # Loop over the keys of each prefix, handling them
        handlePrefix =
          inputName: pair:
          let
            x = keysForInputAndPrefix reflected.${inputName} pair;
          in
          builtins.filter (x: x != null) (
            map (
              key:
              let
                system = if x ? system then x.system else systemIfPresent reflected.${inputName} pair.prefix;

                prefix = selectPrefix reflected.${inputName} pair.prefix;

                processed =
                  if processor != null then
                    let
                      item = attrByPath key null prefix;
                      out = builtins.tryEval (processor prefix key item);
                    in
                    if item != null && out.success != false then
                      out.value
                    else
                      builtins.trace "Found key but is somehow missing" null
                  else
                    null;
              in
              if processor == null || processed != null then
                {
                  inherit key system;
                  explicitSystem = x ? system;
                  prefix = pair.prefix;
                  input = inputName;
                  description = processed.description or null;
                }
              else
                null
            ) x.keys
          );

        # Loop over the prefixes of each input, handling them
        handleInput =
          let
          in
          inputName:
          (map (handlePrefix inputName) (
            builtins.filter (
              p: builtins.elem p.prefix (builtins.attrNames reflected.${inputName})
            ) prefixKeyPairs
          ));

        # Loop over the inputs, handling each one
        handled = map handleInput inputs;
      in
      # Flatten our lists twice (for prefix and input) to get a flat list of matches
      (builtins.concatLists (builtins.concatLists handled));
  };
}
