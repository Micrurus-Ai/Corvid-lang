/* Ordered string maps own a flat buffer of key/value pairs. The descriptor
 * stays at one address as the buffer grows, so aliases observe each update. */
#include "list_layout.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct corvid_typeinfo {
    uint32_t size, flags;
    void (*destroy_fn)(void*);
    void (*trace_fn)(void*, void (*)(void*, void*), void*);
    void (*weak_fn)(void*);
    const struct corvid_typeinfo* elem_typeinfo;
    const char* name;
};

extern void* corvid_alloc_typed(long long, const struct corvid_typeinfo*);
extern void corvid_release(void*);
extern void corvid_retain(void*);
extern long long corvid_string_eq(void*, void*);

typedef struct {
    int64_t count;
    void* slots[];
} map_slots;

typedef struct {
    map_slots* entries;
    int64_t capacity;
    int64_t owns_values;
} ordered_map;

static void destroy_entries(void* payload) {
    map_slots* entries = payload;
    for (int64_t i = 0; i < entries->count * 2; ++i) corvid_release(entries->slots[i]);
}

static void trace_entries(void* payload, void (*mark)(void*, void*), void* ctx) {
    map_slots* entries = payload;
    for (int64_t i = 0; i < entries->count * 2; ++i) mark(entries->slots[i], ctx);
}

static void destroy_keys(void* payload) {
    map_slots* entries = payload;
    for (int64_t i = 0; i < entries->count; ++i) corvid_release(entries->slots[2 * i]);
}

static void trace_keys(void* payload, void (*mark)(void*, void*), void* ctx) {
    map_slots* entries = payload;
    for (int64_t i = 0; i < entries->count; ++i) mark(entries->slots[2 * i], ctx);
}

static void destroy_map(void* payload) {
    corvid_release(((ordered_map*)payload)->entries);
}

static void trace_map(void* payload, void (*mark)(void*, void*), void* ctx) {
    mark(((ordered_map*)payload)->entries, ctx);
}

static const struct corvid_typeinfo entries_info = {
    0, 0, destroy_entries, trace_entries, NULL, NULL, "Map entries"
};
static const struct corvid_typeinfo scalar_entries_info = {
    0, 0, destroy_keys, trace_keys, NULL, NULL, "Map scalar entries"
};
const struct corvid_typeinfo corvid_typeinfo_OrderedMap = {
    sizeof(ordered_map), 0, destroy_map, trace_map, NULL, NULL, "Map"
};

static map_slots* allocate_entries(int64_t capacity, int64_t owns_values) {
    if (capacity < 0 || capacity > (INT64_MAX - 8) / 16) abort();
    map_slots* entries = corvid_alloc_typed(8 + capacity * 16, owns_values ? &entries_info : &scalar_entries_info);
    entries->count = 0;
    return entries;
}

ordered_map* corvid_ordered_map_new_typed(int64_t capacity, int64_t owns_values) {
    ordered_map* map = corvid_alloc_typed(sizeof(ordered_map), &corvid_typeinfo_OrderedMap);
    map->entries = allocate_entries(capacity, owns_values);
    map->owns_values = owns_values;
    map->capacity = capacity;
    return map;
}

/* Existing native objects use the original string-value constructor. */
ordered_map* corvid_ordered_map_new(int64_t capacity) {
    return corvid_ordered_map_new_typed(capacity, 1);
}

/* Return the slot address so absence is distinct from a zero-valued slot. */
void* corvid_ordered_map_find(ordered_map* map, void* key) {
    map_slots* entries = map->entries;
    for (int64_t i = 0; i < entries->count; ++i) {
        if (corvid_string_eq(entries->slots[i * 2], key)) return &entries->slots[i * 2 + 1];
    }
    return NULL;
}

int64_t corvid_ordered_map_require_int(ordered_map* map, void* key) {
    void** slot = corvid_ordered_map_find(map, key);
    if (slot) return (intptr_t)*slot;
    const intptr_t* text = key;
    fputs("corvid: compound assignment requires an existing map key: missing key `", stderr);
    if (text[1] > 0) fwrite((const void*)text[0], 1, (size_t)text[1], stderr);
    fputs("`\n", stderr);
    exit(1);
}

/* Keys and heap values transfer an owned reference. Integer slots are unboxed. */
void corvid_ordered_map_set(ordered_map* map, void* key, void* value) {
    void** slot = corvid_ordered_map_find(map, key);
    if (slot) {
        void* old = *slot;
        *slot = value;
        if (map->owns_values) corvid_release(old);
        corvid_release(key);
        return;
    }
    map_slots* entries = map->entries;
    if (entries->count == map->capacity) {
        int64_t capacity = map->capacity ? map->capacity * 2 : 4;
        map_slots* grown = allocate_entries(capacity, map->owns_values);
        memcpy(grown->slots, entries->slots, entries->count * 16);
        grown->count = entries->count;
        entries->count = 0; /* transfer the stored references */
        map->entries = grown;
        map->capacity = capacity;
        corvid_release(entries);
        entries = grown;
    }
    int64_t i = entries->count++ * 2;
    entries->slots[i] = key;
    entries->slots[i + 1] = value;
}

/* Snapshot into the normal native List<T> layout, retaining heap elements. */
void* corvid_ordered_map_snapshot(ordered_map* map, int64_t values, const struct corvid_typeinfo* list_info) {
    map_slots* entries = map->entries;
    // The caller supplies the result typeinfo. Older native objects use inline slots.
    map_slots* list;
    void** slots;
    if (list_info->flags & CORVID_TI_GROWABLE_LIST) {
        corvid_list* descriptor = corvid_list_new(entries->count, list_info);
        for (int64_t i = 0; i < entries->count; ++i) {
            void* item = entries->slots[2 * i + values];
            if (!values || map->owns_values) corvid_retain(item);
            corvid_list_append(descriptor, (intptr_t)item);
        }
        return descriptor;
    }
    list = corvid_alloc_typed(8 + entries->count * 8, list_info);
    slots = list->slots;
    list->count = 0;
    for (int64_t i = 0; i < entries->count; ++i) {
        void* item = entries->slots[2 * i + values];
        if (!values || map->owns_values) corvid_retain(item);
        slots[i] = item;
        list->count++;
    }
    return list;
}
