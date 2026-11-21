/* Lists have a stable identity across append. Growable descriptors hold the
 * length, data pointer and capacity, followed by inline storage for literals.
 * Older inline-only descriptors are still understood by trace and destroy.
 */
#include "list_layout.h"
#include <stddef.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

#define CORVID_HEADER_BYTES 16

struct corvid_typeinfo {
    uint32_t size;
    uint32_t flags;
    void (*destroy_fn)(void*);
    void (*trace_fn)(void*, void (*marker)(void*, void*), void*);
    void (*weak_fn)(void*);
    const struct corvid_typeinfo* elem_typeinfo;
    const char* name;
};

typedef struct {
    long long refcount_word;
    const struct corvid_typeinfo* typeinfo;
} corvid_header;

extern void corvid_release(void* payload);
extern void corvid_retain(void* payload);
extern void* corvid_alloc_typed(long long bytes, const struct corvid_typeinfo* info);

static const struct corvid_typeinfo* list_info(const void* list) {
    return ((const corvid_header*)list - 1)->typeinfo;
}

static int64_t* list_slots(void* payload) {
    if (list_info(payload)->flags & CORVID_TI_GROWABLE_LIST)
        return ((corvid_list*)payload)->data;
    return (int64_t*)payload + 1;
}

void corvid_list_bounds_error(int64_t index, int64_t length) {
    fprintf(stderr, "corvid: list index %lld out of bounds for length %lld\n",
            (long long)index, (long long)length);
    exit(1);
}

corvid_list* corvid_list_new(int64_t capacity, const struct corvid_typeinfo* info) {
    if (capacity < 0 || capacity > (INT64_MAX - (int64_t)sizeof(corvid_list)) / 8) abort();
    corvid_list* list = corvid_alloc_typed(sizeof(corvid_list) + capacity * 8, info);
    list->length = 0;
    list->capacity = capacity;
    list->data = (int64_t*)(list + 1);
    return list;
}

/* Iteration visits the entries present at loop entry, even if the body
 * mutates the source. Heap entries stay shared at the element level. */
corvid_list* corvid_list_snapshot(corvid_list* source, const struct corvid_typeinfo* info) {
    const int64_t count = source->length;
    corvid_list* result = corvid_list_new(count, info);
    memcpy(result->data, list_slots(source), (size_t)count * sizeof(int64_t));
    if (info->elem_typeinfo) {
        for (int64_t i = 0; i < count; i++) corvid_retain((void*)(intptr_t)result->data[i]);
    }
    result->length = count;
    return result;
}

/* Called for both reference-count destruction and cycle collection. */
void corvid_list_free_storage(void* payload) {
    corvid_list* list = payload;
    if (list->data != (int64_t*)(list + 1)) free(list->data);
}

/* The incoming slot transfers one owned reference, if applicable. */
void corvid_list_append(corvid_list* list, int64_t slot) {
    if (list->length == list->capacity) {
        if (list->capacity > INT64_MAX / 16) abort();
        int64_t capacity = list->capacity ? list->capacity * 2 : 4;
        int64_t* data;
        if (list->data == (int64_t*)(list + 1)) {
            data = malloc((size_t)capacity * 8);
            if (!data) abort();
            memcpy(data, list->data, (size_t)list->length * 8);
        } else {
            data = realloc(list->data, (size_t)capacity * 8);
            if (!data) abort();
        }
        list->data = data;
        list->capacity = capacity;
    }
    list->data[list->length++] = slot;
}

corvid_list* corvid_list_map_int(corvid_list* source, void* closure,
                                 int64_t (*apply)(void*, int64_t),
                                 const struct corvid_typeinfo* info) {
    corvid_list* out = corvid_list_snapshot(source, info);
    const int64_t count = out->length;
    for (int64_t i = 0; i < count; ++i) {
        out->data[i] = apply(closure, out->data[i]);
    }
    return out;
}

corvid_list* corvid_list_filter_int(corvid_list* source, void* closure,
                                    uint8_t (*accept)(void*, int64_t),
                                    const struct corvid_typeinfo* info) {
    corvid_list* out = corvid_list_snapshot(source, info);
    int64_t kept = 0;
    for (int64_t i = 0; i < out->length; ++i) {
        const int64_t item = out->data[i];
        if (accept(closure, item)) out->data[kept++] = item;
    }
    out->length = kept;
    return out;
}

int64_t corvid_list_fold_int(corvid_list* source, void* closure,
                             int64_t (*step)(void*, int64_t, int64_t),
                             const struct corvid_typeinfo* info, int64_t acc) {
    corvid_list* items = corvid_list_snapshot(source, info);
    for (int64_t i = 0; i < items->length; ++i) {
        acc = step(closure, acc, items->data[i]);
    }
    corvid_release(items);
    return acc;
}

uint8_t corvid_list_test_int(corvid_list* source, void* closure,
                             uint8_t (*test)(void*, int64_t),
                             const struct corvid_typeinfo* info, int64_t all) {
    corvid_list* items = corvid_list_snapshot(source, info);
    uint8_t result = (uint8_t)all;
    for (int64_t i = 0; i < items->length; ++i) {
        result = test(closure, items->data[i]);
        if (result != all) break;
    }
    corvid_release(items);
    return result;
}

/* Shared destroy — installed on every refcounted-element list's typeinfo.
 * Walks the length prefix and releases each element. corvid_release
 * then frees the list's own block after this returns.
 */
void corvid_destroy_list(void* payload) {
    long long* p = (long long*)payload;
    long long length = p[0];
    int64_t* slots = list_slots(payload);
    for (long long i = 0; i < length; i++) {
        void* elem = (void*)(intptr_t)slots[i];
        if (elem != NULL) {
            corvid_release(elem);
        }
    }
}

/* Shared trace — installed on every list's typeinfo (refcounted-element
 * AND primitive-element). Invokes `marker(elem, ctx)` for each
 * refcounted element. No-op for primitive-element lists (elem_typeinfo
 * NULL) — prevents the List<Int> mis-trace bug the pre-typed-header
 * design would have hit (tracing Int slots as pointers).
 */
void corvid_trace_list(void* payload,
                       void (*marker)(void*, void*),
                       void* ctx) {
    corvid_header* h = (corvid_header*)((char*)payload - CORVID_HEADER_BYTES);
    if (h->typeinfo == NULL || h->typeinfo->elem_typeinfo == NULL) {
        /* primitive elements — nothing to trace */
        return;
    }
    long long* p = (long long*)payload;
    long long length = p[0];
    int64_t* slots = list_slots(payload);
    for (long long i = 0; i < length; i++) {
        void* elem = (void*)(intptr_t)slots[i];
        if (elem != NULL) {
            marker(elem, ctx);
        }
    }
}
