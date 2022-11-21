{
  description = "Super Resolver";

  inputs = {
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

  outputs = reflected: {
    resolve = {
      inputs,
      prefixes,
      system,
      key,
    }: let
      hasAttrByPath = attrPath: e: let
        attr = builtins.head attrPath;
      in
        if attrPath == []
        then true
        else if e ? ${attr}
        then hasAttrByPath (builtins.tail attrPath) e.${attr}
        else false;

      prefixesForInput = input:
        builtins.filter
        (prefixName: builtins.elem prefixName (builtins.attrNames input))
        prefixes;

      selectPrefix = input: prefixName:
        if input.${prefixName} ? "${system}"
        then input.${prefixName}.${system}
        else input.${prefixName};

      keysForInputAndPrefix = input: prefixName: let
        prefix = selectPrefix input prefixName;
      in
        if key == null
        then map (x: [x]) (builtins.attrNames prefix)
        else if hasAttrByPath key prefix
        then [key]
        else [];

      mayhapsSystem = input: prefixName:
        if input.${prefixName} ? "${system}"
        then "${system}"
        else null;

      handlePrefix = inputName: prefixName:
        map
        (key: {
          inherit key;
          system = mayhapsSystem reflected.${inputName} prefixName;
          prefix = prefixName;
          input = inputName;
        })
        (keysForInputAndPrefix reflected.${inputName} prefixName);

      handleInput = inputName: (
        map
        (handlePrefix inputName)
        (prefixesForInput reflected.${inputName})
      );

      handled =
        map
        handleInput
        inputs;
    in
      builtins.concatLists (builtins.concatLists handled);
  };
}
