/* Synthetic fixture for tools/tests/test_sync_keeperfx.py: the shape of the KeeperFX
 * source file of the same name, with a handful of made-up rows. Not KeeperFX code. */
#ifndef FIXTURE_LVL_SCRIPT_LIB_H
#define FIXTURE_LVL_SCRIPT_LIB_H

#define COMMANDDESC_ARGS_COUNT 8

enum TbScriptCommands {
    Cmd_NONE = 0,
    Cmd_LEVEL_VERSION,
    Cmd_WIN_GAME,
    Cmd_SET_FLAG,
    Cmd_ADD_GOLD_TO_PLAYER,
    Cmd_MAX_CREATURES,
    Cmd_FIXTURE_OLD_BEACON,
    Cmd_RANDOM,
};

#endif
