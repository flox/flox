# Examples Directory

For now I'm housing some of the derivative applications of the SDK in this example sdirectory. They will of course become full fledged repositories once they get a bit of maturity - but for now a monorepo will serve our purposes.

# flox CLI
 
This is currently a prototype replacement for the Bash CLI. It will use clap, the flox_rust_sdk and other libraries to give users a powerful and simple CLI to use to floxify their applications and environments.

# flox daemon

The idea behind the flox daemon is to have a locally running service that can do more advanced tasks than what we want to make available in the CLI. It also could facilitate multiuser environments, distributed tasks, etc.