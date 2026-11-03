#ifndef MEMORY_KERNEL_H
#define MEMORY_KERNEL_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI version, currently 1. */
uint32_t memory_kernel_abi_version(void);

/* Returns 0 after writing exactly 32 raw digest bytes, 1 for invalid arguments,
 * or 2 for a caught Rust panic. Null data is valid only when len == 0.
 * output must hold at least 32 writable bytes; capacity must be >= 32.
 * len must be <= ISIZE_MAX (the platform's signed pointer-sized maximum).
 * Caller owns valid, nonoverlapping buffers for the duration of the call.
 * Input must not be mutated concurrently. No pointers are retained or freed.
 */
int32_t memory_kernel_fingerprint_v1(const uint8_t *data, size_t len,
                                     uint8_t *output, size_t capacity);

#ifdef __cplusplus
}
#endif

#endif
