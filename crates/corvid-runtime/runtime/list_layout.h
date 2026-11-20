#ifndef CORVID_LIST_LAYOUT_H
#define CORVID_LIST_LAYOUT_H

#include <stdint.h>

#define CORVID_TI_GROWABLE_LIST 0x40u

struct corvid_typeinfo;
typedef struct {
    int64_t length;
    int64_t* data;
    int64_t capacity;
} corvid_list;

corvid_list* corvid_list_new(int64_t capacity, const struct corvid_typeinfo* info);
void corvid_list_append(corvid_list* list, int64_t slot);
void corvid_list_free_storage(void* payload);

#endif
