import dspy
from time import sleep
import sys

def search_wikipedia(query: str):
    results = dspy.ColBERTv2(url='http://20.102.90.50:2017/wiki17_abstracts')(query, k=3)
    return [x['text'] for x in results]


async def lookup_population(city: str):
    print("lookup_population called")
    func = getattr(sys.modules[__name__], "_rust_lookup_population", None)
    if func is None:
        raise RuntimeError("Rust thunk not injected!")
    return await func(city)


TOOLS = [
    dspy.Tool(lookup_population),
    search_wikipedia,
]


# Define the DSPy ReAct program
react = dspy.ReAct("question -> next_thought: str, next_tool_name: str, next_tool_args: dict", tools=TOOLS)
react_async = dspy.asyncify(react)