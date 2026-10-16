# messenger-evidence

Evidence for a report on a person in the messenger. A report may attach up
to ten messages; the server of the project that takes it reads nobody's
conversation, yet it checks that the accused really wrote each one, so a
modified client cannot put words in anybody's mouth.

## What a proof is

- **A DM** (NIP-17): the seal of the message — kind 13, signed by its
  author, the encrypted rumor inside — and the NIP-44 v2 keys of that one
  payload (ChaCha20 key and nonce, HMAC key). The client opens the gift
  wrap it received, keeps the seal and derives the keys from the
  conversation key and the payload's nonce (HKDF-expand). The server checks
  the seal's signature, the MAC with the HMAC key, decrypts, and reads the
  rumor: a kind-14 message of the seal's signer, tagged to the other party,
  dated within two days (and an hour) of the seal.
- **A group message**: the inner event (kind 9) signed by its author, with
  the group in its `h` tag; it travels as it is.

`dm_proof` and `group_proof` make proofs on the client, `verify` checks a
report on the server. Neither needs nor learns a key of any user.

## What it proves

"This key wrote this message to that key" (a DM), or "in that group" (a
group message), with the content, time and tags its author signed. The
message may be the accused's, or the reporter's own, attached as context;
the result names whose.

## What it does not prove

- That this is the whole conversation, or what came before or after: the
  reporter picks the messages, and a moderator should read them so.
- That a group message reached the group: anyone who holds it can show it.
- Anything of the reporter's own words in the report (reason, comment):
  nobody but the reporter signed them.

## What the keys reveal

Only the one message they were derived for. HKDF-expand is one way: the
keys of one payload tell nothing of the conversation key, nor of the keys
of any other message. The HMAC key is checked against the MAC the author
signed; the ChaCha20 key is bound by the fact that no other key turns the
signed ciphertext into a well-formed rumor of the same author.

## Tests

The NIP-44 half is checked against the official vectors
(`tests/nip44.vectors.json`, from github.com/paulmillr/nip44). The proofs
are made of the wraps and group messages the app itself makes
(`messenger-dm`, `messenger-groups`), and every forgery in
`tests/proofs.rs` must come out unverified.
