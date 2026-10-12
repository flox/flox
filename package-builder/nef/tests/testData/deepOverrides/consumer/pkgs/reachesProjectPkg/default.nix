# `useBar` is a shallow package in this same repository (not a deep
# override), so it is stubbed exactly like `usesLocalTool.nix`'s
# `localTool`. Confirms the back door a project package could offer
# -- an override reaching `catalogs` transitively by calling a project
# package that itself takes `catalogs` -- is closed: there is no route
# to `useBar`, and therefore none to the `catalogs` it would supply.
{ useBar, ... }: useBar
