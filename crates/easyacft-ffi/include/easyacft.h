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

/* Your own part kinds: declare [kinds.<name>] in the files, place [[custom.<name>]] parts, step them here. */
typedef struct easyacft_part easyacft_part;
typedef void (*easyacft_part_fn)(easyacft_part *p, double dt_s, void *user);
/* Calls `fn` for every unfailed part of `kind` before the systems step; returns how many there are. */
int easyacft_on_kind(easyacft_aircraft *a, const char *kind, easyacft_part_fn fn, void *user);
/* These work only inside a part callback; strings are valid until the next call. */
const char *easyacft_part_id(const easyacft_part *p);
double easyacft_part_get(const easyacft_part *p, const char *value);
int easyacft_part_set(easyacft_part *p, const char *value, double v);
/* Any value of the aircraft, such as "ac1.voltage"; NaN if there is none. */
double easyacft_part_read(const easyacft_part *p, const char *name);
int easyacft_part_write(easyacft_part *p, const char *name, double v);
/* Number, quantity (SI) or true/false setting. */
double easyacft_part_setting(const easyacft_part *p, const char *key);
/* Id of the i-th part a part setting names, or NULL. */
const char *easyacft_part_ref(const easyacft_part *p, const char *key, int i);

/* From your own X-Plane plugin, while the EasyAcft plugin runs the aircraft:
     const easyacft_xp_api *api = NULL;
     XPLMSendMessageToPlugin(XPLMFindPluginBySignature("org.easyacft.plugin"), EASYACFT_MSG_GET_API, &api);
     api->on_kind("my_pump", my_pump_step, NULL);
   Handlers survive hot reloads. Inside a part callback use only the part_ functions. */
#define EASYACFT_MSG_GET_API 0x45410001
typedef struct easyacft_xp_api {
    int version; /* 1 */
    int (*find)(const char *name);
    double (*get)(int handle);
    void (*set)(int handle, double value);
    int (*on_kind)(const char *kind, easyacft_part_fn fn, void *user);
    const char *(*part_id)(const easyacft_part *p);
    double (*part_get)(const easyacft_part *p, const char *value);
    int (*part_set)(easyacft_part *p, const char *value, double v);
    double (*part_read)(const easyacft_part *p, const char *name);
    int (*part_write)(easyacft_part *p, const char *name, double v);
    double (*part_setting)(const easyacft_part *p, const char *key);
    const char *(*part_ref)(const easyacft_part *p, const char *key, int i);
} easyacft_xp_api;

/* Maintenance log: "time part.failed ground|air" lines, oldest first. */
int easyacft_log_count(easyacft_aircraft *a);
const char *easyacft_log_entry(const easyacft_aircraft *a, int i);

#ifdef __cplusplus
}
#endif

#endif
