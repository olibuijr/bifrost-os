# Process accounting report (`lastcomm`)

The command parses Linux `acct_v3` records from `/var/account/pacct`, validates record boundaries and version, expands
compressed CPU times, filters command names, and prints newest records first. It is a reader only: BifrOSt does not
enable kernel accounting or create the accounting file. User IDs are printed numerically, and rotated accounting files
are not read.

Headless unit tests cover little- and big-endian records, flags, time decoding, malformed input and command filtering.
The OS CLI row remains partial until the command is exercised on a machine with accounting enabled.
