#ifndef CIRRA_BRIDGE_H
#define CIRRA_BRIDGE_H
#include <stdint.h>
#include <stddef.h>
typedef struct IosEngine IosEngine;
typedef struct {
    int32_t (*get)(const char *name, uint8_t *output, size_t capacity, size_t *length);
    int32_t (*set)(const char *name, const uint8_t *secret, size_t length);
    int32_t (*delete)(const char *name);
} KeychainCallbacks;
uint32_t cirra_ios_abi_version(void);
IosEngine *cirra_ios_open(const char *directory, KeychainCallbacks callbacks, char **error);
char *cirra_ios_request(IosEngine *, const char *request);
char *cirra_ios_begin_login(IosEngine *);
char *cirra_ios_complete_login(IosEngine *, const char *callback);
void cirra_ios_cancel_login(IosEngine *);
void cirra_ios_close(IosEngine *);
void cirra_ios_free(char *);
#endif
