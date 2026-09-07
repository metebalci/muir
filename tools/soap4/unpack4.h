// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

#pragma once

#include <stdint.h>

size_t unpack(char *filename, uint32_t *half_words, size_t max_len);