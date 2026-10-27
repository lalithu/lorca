from pathlib import Path
import sys
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import outreach_template as template

PROFILE='''# Candidate
## Professional Experience
### Engineering intern
- Built Python API integrations for internal workflows.
## Technical Skills
- **Languages:** Python, Java, Swift.
- **Backend:** FastAPI, PostgreSQL.
## PDF cross-check: unconfirmed
## Technical Skills
- **Unconfirmed:** InventedSkill.
'''


class TemplateTests(unittest.TestCase):
    def setUp(self):
        self.job={'title':'Software Engineer, 2027 New Grad','company':'Whatnot','job_url':'https://example.test/job','description':'Python and FastAPI'}
        self.who={'name':'Ashley Clancy'}

    def render(self,text,profile=PROFILE):return template.render(text,self.job,self.who,{},profile,'initial')

    def test_personal_bracket_template_fills_from_saved_facts(self):
        subject,body=self.render('Hi ___,\nMy name is [NAME]. Interested in a [TYPE OF ROLL] role at [COMPANY]. '+template.EXPERIENCE.join(['[',']'])+'\nMy skills: [LIST A COUPLE SKILLS]. [ROLE] at [COMPANY’S] product/service.\n[ATTACH RESUME]')
        self.assertEqual(subject,'Software Engineer, 2027 New Grad @ Whatnot')
        for expected in ['Hi Ashley,','Lalith Uriti','Python and FastAPI','I built Python API integrations for internal workflows','Whatnot’s']:
            self.assertIn(expected,body)
        self.assertFalse(template.has_markers(body));self.assertNotIn('InventedSkill',body)

    def test_existing_brace_fields_custom_subject_and_wording_are_preserved(self):
        subject,body=self.render('Subject: My {{role}} note\nHello {{first_name}},\nSame wording. [Portfolio](https://example.test)')
        self.assertEqual(subject,'My Software Engineer, 2027 New Grad note')
        self.assertEqual(body,'Hello Ashley,\nSame wording. [Portfolio](https://example.test)')

    def test_missing_facts_and_unknown_markers_never_make_placeholder_drafts(self):
        for text,profile in [('[LIST A COUPLE SKILLS]',''),('['+template.EXPERIENCE+']',''),('[SOMETHING ELSE]',PROFILE),('{{unknown-field}}',PROFILE)]:
            with self.subTest(text=text),self.assertRaises(ValueError):self.render(text,profile)

    def test_custom_subject_does_not_block_greeting_autofill(self):
        subject,body=self.render('Subject: Custom\nHi ___,\n[NAME]')
        self.assertEqual(subject,'Custom');self.assertEqual(body,'Hi Ashley,\nLalith Uriti')
