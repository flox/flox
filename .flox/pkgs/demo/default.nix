{ runCommand }:
runCommand "demo" { } ''
  mkdir -p $out
  echo "there shall be expressions" >> $out/demo
''
