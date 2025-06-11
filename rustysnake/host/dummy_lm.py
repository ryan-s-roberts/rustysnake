import dspy
import random

class DummyLM(dspy.LM):
    def __init__(self, **kwargs):
        super().__init__('dummy', **kwargs)
        self.model = 'dummy'
    def __call__(self, prompt=None, messages=None, signature=None, **kwargs):
        import json
        tool = random.choice(['lookup_population', 'search_wikipedia'])
        if tool == 'lookup_population':
            args = {'city': 'Paris'}
        else:
            args = {'query': 'Eiffel Tower'}
        result = json.dumps({
            'reasoning': 'dummy reasoning',
            'next_thought': 'dummy next_thought',
            'next_tool_name': tool,
            'next_tool_args': args,
        })
        return [result]

dspy.settings.configure(lm=DummyLM(), adapter=dspy.JSONAdapter()) 