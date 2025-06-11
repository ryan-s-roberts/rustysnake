import dspy
import asyncio

class DummyLM(dspy.LM):
    def __init__(self, **kwargs):
        super().__init__('dummy', **kwargs)
        self.model = 'dummy'
    def __call__(self, prompt=None, messages=None, signature=None, **kwargs):
        import json
        # Always return the tool name and args for the test_tool
        return [json.dumps({
            'reasoning': 'dummy reasoning',
            'next_thought': 'dummy next_thought',
            'next_tool_name': 'test_tool',
            'next_tool_args': {'x': 42},
        })]

dspy.settings.configure(lm=DummyLM())

async def test_tool(x):
    print("test_tool called with", x)
    return x + 1

TOOLS = [dspy.Tool(test_tool)]
react = dspy.ReAct("question -> next_tool_name: str, next_tool_args: dict", tools=TOOLS)
react_async = dspy.asyncify(react)

async def main():
    await react_async(question="test?")

if __name__ == "__main__":
    asyncio.run(main()) 