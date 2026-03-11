#!/usr/bin/env python3
"""
Tool: analyse_sentiment

Protocol:
  stdin:  {"texts": ["I love this!", "This is terrible.", "It's okay."]}
  stdout: {"results": [{"text": "...", "sentiment": "positive", "score": 0.92}, ...]}

This is a simple keyword-based demo. Replace with a real model (e.g. transformers,
TextBlob, or an API call) in production.
"""

import json
import sys


POSITIVE_WORDS = {
    "love", "great", "excellent", "amazing", "wonderful", "fantastic",
    "good", "happy", "best", "awesome", "brilliant", "perfect", "nice",
    "superb", "outstanding", "enjoy", "pleased", "glad", "delighted",
}

NEGATIVE_WORDS = {
    "hate", "terrible", "awful", "horrible", "bad", "worst", "poor",
    "disappointed", "disappointing", "mediocre", "disgusting", "dreadful",
    "unacceptable", "broken", "failed", "fail", "useless", "annoying",
}


def analyse(text: str) -> dict:
    words = set(text.lower().split())
    pos = len(words & POSITIVE_WORDS)
    neg = len(words & NEGATIVE_WORDS)
    total = pos + neg

    if total == 0:
        return {"text": text, "sentiment": "neutral", "score": 0.5}

    pos_ratio = pos / total
    if pos_ratio > 0.6:
        sentiment = "positive"
        score = round(0.5 + pos_ratio * 0.5, 2)
    elif pos_ratio < 0.4:
        sentiment = "negative"
        score = round(0.5 + (1 - pos_ratio) * 0.5, 2)
    else:
        sentiment = "neutral"
        score = 0.5

    return {"text": text, "sentiment": sentiment, "score": score}


def main():
    data = json.load(sys.stdin)
    texts = data.get("texts", [])
    if isinstance(texts, str):
        texts = [texts]

    results = [analyse(t) for t in texts]
    json.dump({"results": results, "count": len(results)}, sys.stdout)


if __name__ == "__main__":
    main()
