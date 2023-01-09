{
  description = "Super Resolver";

  inputs = {
    # Empty inputs to be overriden
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
    resolve = {
      # List of string input names, to control which inputs we actually scan
      inputs,
      # List of default prefixnames we scan
      defaultPrefixes,
      # The specified prefix to scan under
      # Might be null
      prefix,
      # The current system to resolve for
      system,
      # A list of strings comprising an attrpath to be applied after the prefix (and system if present) to reach the potential installable
      # Might be null
      key,
      # Internally used to re-execute ourselves setting the key as the prefix when key=[x] and prefix=null
      keyAsPrefix ? false,
      # Used to optionally grab a description from each match
      # List of keys to be used with `attrByPath`
      descriptionKey ? null,
    }: let
      # Copied from nixpkgs
      attrByPath = attrPath: default: set: let
        attr = builtins.head attrPath;
      in
        if attrPath == []
        then set
        else if set ? ${attr}
        then attrByPath (builtins.tail attrPath) default set.${attr}
        else default;
      # Copied from nixpkgs
      hasAttrByPath = attrPath: e: let
        attr = builtins.head attrPath;
      in
        if attrPath == []
        then true
        else if e ? ${attr}
        then hasAttrByPath (builtins.tail attrPath) e.${attr}
        else false;

      # List of prefixes that we are looking for that are present on the input
      prefixesForInput = input:
        builtins.filter
        (prefixName: builtins.elem prefixName (builtins.attrNames input))
        (
          if prefix != null
          then [prefix]
          else defaultPrefixes
        );

      # Get the prefix from an input by name, accounting for systemization if present
      selectPrefix = input: prefixName:
        if input.${prefixName} ? "${system}"
        then input.${prefixName}.${system}
        else input.${prefixName};

      # Create a list of all matching keys (remember keys are a list of strings too).
      keysForInputAndPrefix = input: prefixName: let
        prefix = selectPrefix input prefixName;
      in
        # If no key, all available keys
        if key == null
        then {keys = map (x: [x]) (builtins.attrNames prefix);}
        # If provided key matches on systemized prefix, just the key
        else if hasAttrByPath key prefix
        then {keys = [key];}
        # If provided key matches on prefix without system, use the first component of the key as a system
        else if systemIfPresent input prefixName != null && hasAttrByPath key input.${prefixName}
        then {
          keys =
            if builtins.length key >= 2
            # If key is long enough to include a tail, then the tail can be the key
            then [(builtins.tail key)]
            # If key only includes the system, then all available keys under than system
            else map (x: [x]) (builtins.attrNames input.${prefixName}.${builtins.head key});
          system = builtins.head key;
        }
        # Else no key matches
        else {keys = [];};

      # The current system to resolve for if a system follows the prefix name on the input
      systemIfPresent = input: prefixName:
        if input.${prefixName} ? "${system}"
        then "${system}"
        else null;

      # Loop over the keys of each prefix, handling them
      handlePrefix = inputName: prefixName: let
        x = keysForInputAndPrefix reflected.${inputName} prefixName;
      in
        map
        (key: rec {
          inherit key;
          explicitSystem = x ? system;
          system =
            if x ? system
            then x.system
            else systemIfPresent reflected.${inputName} prefixName;
          prefix = prefixName;
          input = inputName;
          description =
            if descriptionKey != null
            then let
              prefixed =
                if system == null
                then reflected.${inputName}.${prefix}
                else reflected.${inputName}.${prefix}.${system};
              item = attrByPath key null prefixed;
            in
              if item != null
              then attrByPath descriptionKey null item
              else builtins.trace "Found key but is missing" null
            else null;
        })
        x.keys;

      # Loop over the prefixes of each input, handling them
      handleInput = let
      in
        inputName: (
          map
          (handlePrefix inputName)
          (prefixesForInput reflected.${inputName})
        );

      # Loop over the inputs, handling each one
      handled =
        map
        handleInput
        inputs;
    in
      # Flatten our lists twice (for prefix and input) to get a flat list of matches
      (builtins.concatLists (builtins.concatLists handled))
      ++ (
        if keyAsPrefix == false && prefix == null && key != null && builtins.length key == 1
        then
          resolve {
            inherit inputs defaultPrefixes system;
            prefix = builtins.elemAt key 0;
            key = null;
            keyAsPrefix = true;
          }
        else []
      );
  };
}
