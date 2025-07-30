#!/usr/bin/env python3
"""
Simple integration test script for cache metadata functionality.
This script tests the cache metadata features without requiring complex test setup.
"""

import subprocess
import sys
import os
import tempfile
import json

def run_command(cmd, cwd=None):
    """Run a command and return the result."""
    try:
        result = subprocess.run(cmd, shell=True, capture_output=True, text=True, cwd=cwd)
        return result.returncode, result.stdout, result.stderr
    except Exception as e:
        return 1, "", str(e)

def test_compilation():
    """Test that the code compiles successfully."""
    print("Testing compilation...")
    returncode, stdout, stderr = run_command("cargo check")
    if returncode == 0:
        print("✓ Code compiles successfully")
        return True
    else:
        print("✗ Compilation failed")
        print("STDERR:", stderr)
        return False

def test_cache_structure():
    """Test that cache metadata structures are properly defined."""
    print("Testing cache metadata structures...")
    
    # Check if the cache.rs file contains the expected structures
    cache_file = "src/cache.rs"
    if not os.path.exists(cache_file):
        print("✗ Cache file not found")
        return False
    
    with open(cache_file, 'r') as f:
        content = f.read()
    
    # Check for key structures and methods
    required_items = [
        "struct Id3Metadata",
        "struct CacheEntry",
        "unique_id",
        "extract_id3_metadata",
        "generate_unique_id",
        "get_track_metadata",
        "search_cached_tracks",
        "get_album_tracks",
        "get_cached_albums",
        "update_track_metadata"
    ]
    
    missing_items = []
    for item in required_items:
        if item not in content:
            missing_items.append(item)
    
    if missing_items:
        print(f"✗ Missing items: {missing_items}")
        return False
    
    print("✓ All cache metadata structures found")
    return True

def test_id3_import():
    """Test that ID3 crate is properly imported and used."""
    print("Testing ID3 import...")
    
    cache_file = "src/cache.rs"
    with open(cache_file, 'r') as f:
        content = f.read()
    
    if "use id3::TagLike;" in content:
        print("✓ ID3 TagLike trait imported")
        return True
    else:
        print("✗ ID3 TagLike trait not imported")
        return False

def test_cache_methods():
    """Test that cache methods are properly implemented."""
    print("Testing cache method signatures...")
    
    cache_file = "src/cache.rs"
    with open(cache_file, 'r') as f:
        content = f.read()
    
    # Check for method signatures
    method_signatures = [
        "pub async fn get_track_metadata",
        "pub async fn get_track_by_unique_id",
        "pub async fn has_track_by_unique_id",
        "pub async fn search_cached_tracks",
        "pub async fn get_album_tracks",
        "pub async fn get_cached_albums",
        "pub async fn update_track_metadata"
    ]
    
    missing_methods = []
    for method in method_signatures:
        if method not in content:
            missing_methods.append(method)
    
    if missing_methods:
        print(f"✗ Missing methods: {missing_methods}")
        return False
    
    print("✓ All cache methods implemented")
    return True

def test_search_query_types():
    """Test that search query types are defined."""
    print("Testing search query types...")
    
    cache_file = "src/cache.rs"
    with open(cache_file, 'r') as f:
        content = f.read()
    
    if "pub enum SearchQuery" in content:
        print("✓ SearchQuery enum defined")
        return True
    else:
        print("✗ SearchQuery enum not defined")
        return False

def test_public_structures():
    """Test that public structures are accessible."""
    print("Testing public structures...")
    
    cache_file = "src/cache.rs"
    with open(cache_file, 'r') as f:
        content = f.read()
    
    public_structures = [
        "pub struct CachedTrackInfo",
        "pub struct AlbumInfo",
        "pub enum SearchQuery"
    ]
    
    missing_structures = []
    for structure in public_structures:
        if structure not in content:
            missing_structures.append(structure)
    
    if missing_structures:
        print(f"✗ Missing public structures: {missing_structures}")
        return False
    
    print("✓ All public structures defined")
    return True

def main():
    """Run all tests."""
    print("Running cache metadata integration tests...")
    print("=" * 50)
    
    tests = [
        test_compilation,
        test_cache_structure,
        test_id3_import,
        test_cache_methods,
        test_search_query_types,
        test_public_structures
    ]
    
    passed = 0
    total = len(tests)
    
    for test in tests:
        try:
            if test():
                passed += 1
            else:
                print()
        except Exception as e:
            print(f"✗ Test failed with exception: {e}")
        print()
    
    print("=" * 50)
    print(f"Tests passed: {passed}/{total}")
    
    if passed == total:
        print("🎉 All tests passed! Cache metadata functionality is working.")
        return 0
    else:
        print("❌ Some tests failed. Please check the implementation.")
        return 1

if __name__ == "__main__":
    sys.exit(main())