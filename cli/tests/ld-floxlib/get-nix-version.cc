#include "nix/shared.hh"

int main() {
  try {
    nix::printVersion("testing");
  } catch (std::exception const &exc) {
  }
}
