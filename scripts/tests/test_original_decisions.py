import importlib.util,pathlib,unittest
spec=importlib.util.spec_from_file_location('original',pathlib.Path(__file__).parents[1]/'score-original-decisions.py');module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
class OriginalDecisionTests(unittest.TestCase):
 def test_corrections_do_not_erase_errors_and_duplicate_plies_are_errors(self):
  reference=[{'uci':'e2e4'},{'uci':'e7e5'}];decisions=[{'ply':1,'uci':'d2d4','revision':0,'provenance':'automatic'},{'ply':1,'uci':'e2e4','revision':1,'provenance':'reviewed'},{'ply':2,'uci':'e7e5','revision':1,'provenance':'automatic'},{'ply':2,'uci':'e7e5','revision':2,'provenance':'automatic'}]
  r=module.score(reference,{'original_decisions':decisions});self.assertEqual(r['automatic_errors'],2);self.assertEqual(r['coverage'],.5);self.assertEqual(r['precision'],1/3);self.assertFalse(r['unattended_exact_game']);self.assertFalse(r['release_qualified'])
 def test_empty_denominators_stay_unknown(self):
  r=module.score([],{'original_decisions':[]});self.assertIsNone(r['precision']);self.assertIsNone(r['coverage']);self.assertIsNone(r['unattended_exact_game'])
