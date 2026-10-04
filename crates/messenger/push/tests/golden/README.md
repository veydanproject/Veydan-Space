# Examples of the protocol, word for word

These files are the contract between VPush and its clients. The server's
tests read them; a client keeps a copy of this folder and its tests read the
copy. When a file here changes, the copies must change with it, and a check
in CI compares them.

A change that an older client cannot read is a new version of the protocol,
not an edit of these files.
