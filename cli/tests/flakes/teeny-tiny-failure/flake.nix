{
  outputs = {
    self,
    ...
  }:
  {
    packages.aarch64-darwin.default = throw "I'm broken inside";
    packages.aarch64-linux.default = throw "I'm broken inside";
    packages.x86_64-darwin.default = throw "I'm broken inside";
    packages.x86_64-linux.default = throw "I'm broken inside";
  };
}
