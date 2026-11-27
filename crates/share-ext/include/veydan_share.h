/* SPDX-FileCopyrightText: 2026 Veydan Project */
/* SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 */

/*
 * The messenger inside the share extension of the iPhone (crates/share-ext,
 * src/lib.rs): opens the app's data file in the container of the app
 * group, lists the chats a share may go to, sends what another app shared
 * and waits until it is out. One session at a time; every call is made
 * from one thread of the extension, never the main one (a send blocks).
 */

#ifndef VEYDAN_SHARE_H
#define VEYDAN_SHARE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* What `veydan_share_open` answers. */
enum {
  VEYDAN_SHARE_OK = 0,
  /* The lock of the app is set and the secret given opens it not (or none was given): ask for the PIN. */
  VEYDAN_SHARE_LOCKED = 1,
  /* The app has no key of the messenger yet: nothing can be sent. */
  VEYDAN_SHARE_NO_IDENTITY = 2,
  /* Anything else; `veydan_share_error` says what. */
  VEYDAN_SHARE_ERROR = 3,
};

/* The stage a send is at, for the progress of the extension. */
enum {
  VEYDAN_SHARE_STAGE_PREPARING = 1,
  /* `done` and `total` are bytes of the files. */
  VEYDAN_SHARE_STAGE_UPLOADING = 2,
  /* The files are up; the messages go to the relay. */
  VEYDAN_SHARE_STAGE_SENDING = 3,
  VEYDAN_SHARE_STAGE_DONE = 4,
};

/* Opens the data at `data_dir` (`<group container>/veydan/data`), with
 * `secret` (the PIN or password of the app's lock) when the lock is set,
 * NULL otherwise. Starts the messenger. */
int32_t veydan_share_open(const char *data_dir, const char *secret);

/* The chats a share may go to, newest first, as JSON:
 * [{"id","title","kind","last_message_at"}]. NULL on error. Free with `veydan_share_free`. */
char *veydan_share_chats(void);

/* Sends `text` (may be NULL) and the files `paths_json` (a JSON array of
 * paths, may be NULL or empty) into the chat `chat_id`, and returns once
 * they are out (or failed: VEYDAN_SHARE_ERROR). `progress` is called from
 * the thread of the call with the stage and, while uploading, the bytes. */
int32_t veydan_share_send(const char *chat_id, const char *text, const char *paths_json,
                          void (*progress)(void *ctx, int32_t stage, uint64_t done, uint64_t total), void *ctx);

/* The last error, in words; NULL when there is none. Free with `veydan_share_free`. */
char *veydan_share_error(void);

/* Stops the messenger and closes the data file. */
void veydan_share_close(void);

void veydan_share_free(char *s);

#ifdef __cplusplus
}
#endif

#endif
