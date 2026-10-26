import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import sys
import uuid
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import jobtracker as tracker
import resumes
import shared_settings
import google_connections as google

SOURCE=r'''\documentclass{article}
\newcommand{\resumeItem}[1]{\item #1}
\newcommand{\resumeItemListStart}{\begin{itemize}}
\newcommand{\resumeItemListEnd}{\end{itemize}}
\begin{document}
\resumeItemListStart
\resumeItem{Python API with \textbf{PostgreSQL}.}
\resumeItem{Built a frontend with React.}
\resumeItemListEnd
\end{document}
'''

class ResumeTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name)
        self.patches=[patch.object(tracker,'ROOT',self.root),patch.object(tracker,'DB_PATH',self.root/'jobs.sqlite3'),patch.object(google,'ROOT',self.root),patch.object(google,'CONFIG',self.root/'google.json')]
        for item in self.patches:item.start()
        tracker.init_db();resumes.init();self.job=tracker.create_job({'company':'Example','title':'Backend engineer'})
    def tearDown(self):
        for item in reversed(self.patches):item.stop()
        self.tmp.cleanup()
    def test_reorder_handles_nested_latex_and_preserves_exact_facts(self):
        self.assertEqual(len(resumes.groups(SOURCE)),1)
        changed=resumes.reorder(SOURCE,[[1,0]])
        self.assertLess(changed.index('Built a frontend'),changed.index('Python API'))
        self.assertIn(r'\textbf{PostgreSQL}',changed)
        for invalid in ([[1,1]],[[0]],[[0,1,2]],[],[['0',1]]):
            with self.assertRaises(ValueError):resumes.reorder(SOURCE,invalid)
    def test_shared_source_cannot_inject_paths_or_invalid_versions(self):
        row={'source':SOURCE,'master_source':SOURCE,'version':str(uuid.uuid4()),'selection':'tailored','saved_version':'','submitted_version':'','updated_at':tracker.now()}
        with tracker.db() as db:resumes.import_record(db,self.job['id'],json.dumps(row))
        self.assertEqual(json.loads(resumes.portable(self.job['id']))['source'],SOURCE)
        for key,value in [('version','../../other'),('selection','send'),('pdf_path','/etc/passwd')]:
            with tracker.db() as db:
                with self.assertRaises(ValueError):resumes.import_record(db,self.job['id'],json.dumps({**row,key:value}))
    def test_applied_requires_the_current_selected_resume_version(self):
        google.save_config({'resume_sha256':'master-version'})
        with self.assertRaises(ValueError):resumes.applied(self.job['id'],'stale')
        resumes.applied(self.job['id'],'master-version')
        self.assertEqual(tracker.get_job(self.job['id'])['stage'],'Applied')
    def test_pdftex_compatibility_is_only_in_compilation_copy(self):
        text='\\input{glyphtounicode}\n\\pdfgentounicode=1'
        compiled=resumes.compatible(text)
        self.assertIn('\\ifdefined\\pdfglyphtounicode',compiled)
        self.assertEqual(text,'\\input{glyphtounicode}\n\\pdfgentounicode=1')
