# Examples of the protocol, word for word

These files are the contract between VPush and its clients. The server's
tests read them; a client keeps a copy of this folder and its tests read the
copy. When a file here changes, the copies must change with it, and a check
in CI compares them.

A change that an older client cannot read is a new version of the protocol,
not an edit of these files.

The examples of the gateway (`spec/protocol.md`, "The gateway") seal a push
with keys made for them alone: the push key of the device is the secret
`1111…11` (64 hex, public `4f355bdc…`), the key of the private push server
the secret `2222…22` (public `466d7fca…`, the `sender`). The `blob` of
`wake.json` and `push_sealed.json` opens with them to the `data` map of
`push_group.json`, the push FCM would have carried, every value a string:

```json
{"count":"3","event_id":"4444…44","group_id":"1111…11","relay":"wss://node-1.veydan.net","trace":"9583cb58","type":"group","v":"2"}
```

`wake_urgent.json` is the wake of a call, `"urgent": true`: its `blob` opens
with the same keys to the push of `push_dm.json` marked as a call,
`"call": "1"` before the other keys. `wake.json` has no `urgent`, which
reads as `false`.
