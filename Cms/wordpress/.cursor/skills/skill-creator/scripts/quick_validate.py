#!/usr/bin/env python3
"""
Quick validation script for skills - minimal version
"""

import sys
import os
import re
import ast
from pathlib import Path

try:
    import yaml
except ImportError:  # Keep project skill validation usable without PyYAML.
    yaml = None


def parse_frontmatter(frontmatter_text):
    if yaml is not None:
        return yaml.safe_load(frontmatter_text)

    parsed = {}
    for line in frontmatter_text.splitlines():
        if not line or line[0].isspace() or ':' not in line:
            continue
        key, raw_value = line.split(':', 1)
        value = raw_value.strip()
        if not value:
            parsed[key] = {}
        elif value in {'null', '~'}:
            parsed[key] = None
        elif value.startswith('[') and value.endswith(']'):
            items = value[1:-1].strip()
            parsed[key] = (
                []
                if not items
                else [item.strip().strip("\"'") for item in items.split(',')]
            )
        elif value[0:1] in {'"', "'"}:
            parsed[key] = ast.literal_eval(value)
        else:
            parsed[key] = value
    return parsed


def is_valid_skill_name(value):
    return (
        isinstance(value, str)
        and len(value) <= 64
        and re.fullmatch(r'[a-z0-9]+(?:-[a-z0-9]+)*', value) is not None
    )


def validate_skill(skill_path):
    """Basic validation of a skill"""
    skill_path = Path(skill_path)

    # Check SKILL.md exists
    skill_md = skill_path / 'SKILL.md'
    if not skill_md.exists():
        return False, "SKILL.md not found"

    # Read and validate frontmatter
    content = skill_md.read_text()
    if not content.startswith('---'):
        return False, "No YAML frontmatter found"

    # Extract frontmatter
    match = re.match(r'^---\n(.*?)\n---', content, re.DOTALL)
    if not match:
        return False, "Invalid frontmatter format"

    frontmatter_text = match.group(1)

    # Parse YAML frontmatter
    try:
        frontmatter = parse_frontmatter(frontmatter_text)
        if not isinstance(frontmatter, dict):
            return False, "Frontmatter must be a YAML dictionary"
    except Exception as e:
        return False, f"Invalid YAML in frontmatter: {e}"

    # Define allowed properties
    ALLOWED_PROPERTIES = {
        'name', 'description', 'license', 'allowed-tools', 'metadata',
        'compatibility', 'phase', 'flow-next', 'flow-alternatives',
    }

    # Check for unexpected properties (excluding nested keys under metadata)
    unexpected_keys = set(frontmatter.keys()) - ALLOWED_PROPERTIES
    if unexpected_keys:
        return False, (
            f"Unexpected key(s) in SKILL.md frontmatter: {', '.join(sorted(unexpected_keys))}. "
            f"Allowed properties are: {', '.join(sorted(ALLOWED_PROPERTIES))}"
        )

    # Check required fields
    if 'name' not in frontmatter:
        return False, "Missing 'name' in frontmatter"
    if 'description' not in frontmatter:
        return False, "Missing 'description' in frontmatter"

    if 'phase' in frontmatter:
        if frontmatter['phase'] not in {
            'planning', 'execution', 'quality', 'utility'
        }:
            return False, "'phase' must be planning, execution, quality, or utility"

    if 'flow-next' in frontmatter:
        flow_next = frontmatter['flow-next']
        if flow_next is not None and (
            not isinstance(flow_next, str)
            or not is_valid_skill_name(flow_next)
        ):
            return False, "'flow-next' must be null or a kebab-case skill name"

    if 'flow-alternatives' in frontmatter:
        alternatives = frontmatter['flow-alternatives']
        if not isinstance(alternatives, list) or any(
            not is_valid_skill_name(item)
            for item in alternatives
        ):
            return False, "'flow-alternatives' must be a list of kebab-case skill names"

    # Extract name for validation
    name = frontmatter.get('name', '')
    if not isinstance(name, str):
        return False, f"Name must be a string, got {type(name).__name__}"
    name = name.strip()
    if name:
        if len(name) > 64:
            return False, f"Name is too long ({len(name)} characters). Maximum is 64 characters."
        if not is_valid_skill_name(name):
            return False, f"Name '{name}' should be kebab-case"

    # Extract and validate description
    description = frontmatter.get('description', '')
    if not isinstance(description, str):
        return False, f"Description must be a string, got {type(description).__name__}"
    description = description.strip()
    if description:
        # Check for angle brackets
        if '<' in description or '>' in description:
            return False, "Description cannot contain angle brackets (< or >)"
        # Check description length (max 1024 characters per spec)
        if len(description) > 1024:
            return False, f"Description is too long ({len(description)} characters). Maximum is 1024 characters."

    # Validate compatibility field if present (optional)
    compatibility = frontmatter.get('compatibility', '')
    if compatibility:
        if not isinstance(compatibility, str):
            return False, f"Compatibility must be a string, got {type(compatibility).__name__}"
        if len(compatibility) > 500:
            return False, f"Compatibility is too long ({len(compatibility)} characters). Maximum is 500 characters."

    return True, "Skill is valid!"

if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python quick_validate.py <skill_directory>")
        sys.exit(1)
    
    valid, message = validate_skill(sys.argv[1])
    print(message)
    sys.exit(0 if valid else 1)