## Call stack
- lockRegistry
- FlakeRegistry::getLockedInputs
- FloxFlakeInput::getLockedInput
- FloxFlakeInput::getFlake
- FloxFlake::FloxFlake
- lockFlakeWithRightFlags
- lockFlake
- Input::toURLString
- Input::toURL
- Input::scheme is missing

## FloxFlakeInput::getLockedInput
- FloxFlakeInput derives from RegistryInput
- contains a shared_ptr<FloxFlake>: `flake`
- _also_ contains a shared_ptr<FlakeRef> that it inherits from RegistryInput: `from`
- calls getFlake so that it can return the locked flake reference

## FloxFlakeInput::getFlake
- This lazily initializes the FloxFlake if it hasn't been initialized yet
- When hitting the error the FloxFlake hasn't been initialized yet
- When constructing the FloxFlake we pass it the flake reference that's obtained via the inherited method RegistryInput::getFlakeRef
- At this point the scheme is already missing from the flake ref's input
- Let's see how this registry input is created

## lockRegistry
- takes a RegistryRaw and a Nix store
- RegistryRaw contains a map of names to RegistryInputs
- How do we get there?

## Environment::createLockfile
- we end up in Environment::getCombinedRegistryRaw
- since we don't have a global manifest in this test we go straight to Environment::getManifest::getLockedRegistry
- Deep within the inheritance hierarchy you find that ManifestBase contains a RegistryRaw, which again contains a map of names to RegistryInputs
- This means that we probably need to see how those registry inputs are created in the ManifestRaw

## from_json -> RegistryRaw
- The original registry is made all the way at the beginning of the test file when it's converted from JSON into a RegistryRaw
- Calls `value.get_to(input)`, so we need to see the `from_json` for RegistryInputs

## from_json -> RegistryInput
- `value.get<nix::FlakeRef>` as part of this call, so there's code somewhere initializing a flake ref from JSON

## from_json -> nix::FlakeRef
- We have an adl_serializer in `util.hh` that defines this
- It calls `nix::FlakeRef::fromAttrs( nix::fetchers::jsonToAttrs(jfrom))`
- I split the call apart so I can inspect it

## FlakeRef::fromAttrs
- Calls Input::fromAttrs
- no debugging symbols
- F
