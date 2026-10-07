def rerank(items: list[int]) -> list[int]:
    """Order search results by score, highest first."""
    return sorted(items, reverse=True)
