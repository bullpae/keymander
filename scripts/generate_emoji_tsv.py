#!/usr/bin/env python3
"""Generate emoji.tsv from Unicode emoji-test.txt"""
import urllib.request
import re
import os

url = "https://unicode.org/Public/emoji/16.0/emoji-test.txt"
req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 (compatible; keymander/1.0)"})
response = urllib.request.urlopen(req)
content = response.read().decode('utf-8')

group = ""
subgroup = ""
entries = []

for line in content.split('\n'):
    line_stripped = line.strip()

    # Track group
    if line_stripped.startswith('# group:'):
        group = line_stripped.split(':', 1)[1].strip()
        subgroup = ""
        continue

    # Track subgroup
    if line_stripped.startswith('# subgroup:'):
        subgroup = line_stripped.split(':', 1)[1].strip()
        continue

    # Skip comments and empty lines
    if not line_stripped or line_stripped.startswith('#'):
        continue

    # Only fully-qualified
    if 'fully-qualified' not in line_stripped:
        continue

    # Skip ZWJ sequences (variants of base emoji)
    if '200D' in line_stripped.split(';')[0]:
        continue

    # Parse: "1F600 ; fully-qualified # 😀 E1.0 grinning face"
    match = re.match(
        r'^[\dA-F ]+;\s*fully-qualified\s*#\s*(\S+)\s+E[\d.]+\s+(.+)$',
        line_stripped
    )
    if not match:
        continue

    emoji = match.group(1)
    name = match.group(2).strip()

    # Skip skin tone variants
    if 'skin tone' in name.lower():
        continue

    # Skip hair style and other modifier variants (": " followed by descriptors)
    if ': ' in name:
        continue

    category = f"{group}: {subgroup}" if subgroup else group
    entries.append(f"{emoji}\t{name}\t{category}")

# Ensure data directory exists
data_dir = os.path.join(os.path.dirname(__file__), '..', 'crates', 'kmd-core', 'data')
os.makedirs(data_dir, exist_ok=True)

output_path = os.path.join(data_dir, 'emoji.tsv')
with open(output_path, 'w', encoding='utf-8') as f:
    for entry in entries:
        f.write(entry + '\n')

print(f"Generated {len(entries)} emoji entries to {output_path}")
