/* EasyAcft C API. */
#ifndef EASYACFT_H
#define EASYACFT_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct easyacft_aircraft easyacft_aircraft;

/* NULL on failure; `errors` (may be NULL) then gets a message for easyacft_free_string. */
easyacft_aircraft *easyacft_load(const char *dir, char **errors);
void easyacft_free(easyacft_aircraft *a);
void easyacft_free_string(char *s);

/* Advances `dt_s` seconds of sim time. */
void easyacft_step(easyacft_aircraft *a, double dt_s);

/* Handle of a value such as "dc1.voltage", or -1. */
int easyacft_find(const easyacft_aircraft *a, const char *name);
double easyacft_get(const easyacft_aircraft *a, int handle);
void easyacft_set(easyacft_aircraft *a, int handle, double value);
int easyacft_count(const easyacft_aircraft *a);
/* Valid until the aircraft is freed. */
const char *easyacft_name(const easyacft_aircraft *a, int handle);

/* Copies a snapshot into `buf` if it fits; returns its size either way. */
size_t easyacft_snapshot(const easyacft_aircraft *a, unsigned char *buf, size_t len);
int easyacft_restore(easyacft_aircraft *a, const unsigned char *buf, size_t len);

typedef void (*easyacft_step_fn)(easyacft_aircraft *a, double dt_s, void *user);
/* Calls `fn` after every step: the hook for custom parts and controllers. */
void easyacft_on_step(easyacft_aircraft *a, easyacft_step_fn fn, void *user);
/* Like easyacft_on_step, but only while computer `id` is powered, booted and healthy; 0 if there is no such computer. */
int easyacft_on_computer_step(easyacft_aircraft *a, const char *id, easyacft_step_fn fn, void *user);

/* Maintenance log: "time part.failed ground|air" lines, oldest first. */
int easyacft_log_count(easyacft_aircraft *a);
const char *easyacft_log_entry(const easyacft_aircraft *a, int i);

#ifdef __cplusplus
}
#endif

#endif
