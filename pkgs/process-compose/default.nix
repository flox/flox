{ process-compose }:

process-compose.overrideAttrs (oldAttrs: {
  patches = oldAttrs.patches or [ ] ++ [
    # Disable a warning message if no user/global process-compose
    # config dir is found.
    # Previously the config was only looked for when interacting with the TUI[1],
    # but seems to be accessed (loudly) for any process-compose command now[2].
    # [1]: https://github.com/F1bonacc1/process-compose/issues/175
    # [2]: https://github.com/F1bonacc1/process-compose/issues/366
    ./process-compose-warning-message.patch
  ];
})
