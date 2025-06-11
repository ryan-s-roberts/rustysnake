import time
import random
import dspy
from time import sleep
import sys

def search_wikipedia(query: str):
    results = dspy.ColBERTv2(url='http://20.102.90.50:2017/wiki17_abstracts')(query, k=3)
    return [x['text'] for x in results]

def lookup_population(city: str):
    # Always fetch the latest function from the module (including Rust thunk)
    return getattr(sys.modules[__name__], "lookup_population")(city)

def _lookup_population_placeholder(city: str):
    # Placeholder, will be replaced by Rust thunk from Rust host
    pass
# Register tools for DSPy
TOOLS = [
    lookup_population,   # This function's __name__ is 'lookup_population'
    search_wikipedia,
]

# Define the DSPy ReAct program
react = dspy.ReAct("question -> next_thought: str, next_tool_name: str, next_tool_args: dict", tools=TOOLS)