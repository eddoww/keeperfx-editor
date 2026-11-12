/* Synthetic fixture for tools/tests/test_sync_keeperfx.py: the shape of the KeeperFX
 * source file of the same name, with a handful of made-up rows. Not KeeperFX code. */
#include "lvl_script_lib.h"

const struct NamedCommand player_desc[] = {
  {"PLAYER0",          0},
  {"PLAYER1",          1},
  {"PLAYER_GOOD",      4},
  {"ALL_PLAYERS",      8},
  {NULL,               0},
};

const struct NamedCommand flag_desc[] = {
  {"FLAG0",            0},
  {"FLAG1",            1},
  {NULL,               0},
};

const struct NamedCommand timer_desc[] = {
  {"TIMER0",           0},
  {NULL,               0},
};

/**
 * Descriptions of script commands for parser.
 * Arguments are: A-string, N-integer, C-creature model, P-player, R-room kind, L-location, O-operator, S-slab kind, B-boolean
 * Lower case letters are optional arguments.
 */
const struct CommandDesc command_desc[] = {
  {"LEVEL_VERSION",       "N       ", Cmd_LEVEL_VERSION, NULL, NULL},
  {"WIN_GAME",            "        ", Cmd_WIN_GAME, NULL, NULL},
  {"SET_FLAG",            "PAN     ", Cmd_SET_FLAG, NULL, NULL},
  {"ADD_GOLD_TO_PLAYER",  "PN      ", Cmd_ADD_GOLD_TO_PLAYER, NULL, NULL},
  {"MAX_CREATURES",       "PN      ", Cmd_MAX_CREATURES, NULL, NULL},
  {"FIXTURE_OLD_BEACON",  "PN      ", Cmd_FIXTURE_OLD_BEACON, NULL, NULL},
  {NULL,                  "        ", Cmd_NONE, NULL, NULL},
};

const struct CommandDesc dk1_command_desc[] = {
  {"LEVEL_VERSION",       "N       ", Cmd_LEVEL_VERSION, NULL, NULL},
  {"WIN_GAME",            "        ", Cmd_WIN_GAME, NULL, NULL},
  {"SET_FLAG",            "PAN     ", Cmd_SET_FLAG, NULL, NULL},
  {"FIXTURE_OLD_BEACON",  "PN      ", Cmd_FIXTURE_OLD_BEACON, NULL, NULL},
  {NULL,                  "        ", Cmd_NONE, NULL, NULL},
};

const struct CommandDesc subfunction_desc[] = {
  {"RANDOM",              "Aaaaaaaa", Cmd_RANDOM, NULL, NULL},
  {NULL,                  "        ", Cmd_NONE, NULL, NULL},
};
