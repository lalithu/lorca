import importlib.util
from pathlib import Path
import signal
import tempfile
import unittest
from unittest.mock import Mock, patch, call

spec = importlib.util.spec_from_file_location('restart', Path(__file__).resolve().parents[2] / 'restart.py')
restart = importlib.util.module_from_spec(spec)
spec.loader.exec_module(restart)


class RestartTests(unittest.TestCase):
    def test_mapped_executable_requires_exact_inode(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'lorca-core'
            path.write_bytes(b'new build')
            inode = path.stat().st_ino
            with patch.object(restart, 'output', return_value=f'p10\nftxt\ni{inode + 1}\n'):
                self.assertFalse(restart.mapped_to(10, path))
            with patch.object(restart, 'output', return_value=f'p10\nftxt\ni{inode}\n'):
                self.assertTrue(restart.mapped_to(10, path))

    def test_ambiguous_or_unrecognized_backend_is_not_signaled(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'runtime').mkdir()
            (root / 'runtime/backend.lock').touch()
            with patch.object(restart, 'output', return_value='10\n11\n'):
                with self.assertRaisesRegex(RuntimeError, 'Multiple processes'):
                    restart.backend_pid(root)
            with patch.object(restart, 'output', return_value='10\n'), patch.object(restart, 'executable', return_value=Path('/usr/bin/other')):
                with self.assertRaisesRegex(RuntimeError, 'not a recognized Lorca'):
                    restart.backend_pid(root)

    def test_restart_orders_shutdown_backend_start_ui_start_and_live_verification(self):
        app = Path('/private/tmp/restart-test/lorca.app')
        ui = app / 'Contents/MacOS/lorca'
        events = []
        living = {11: True, 12: True}
        def kill(pid, sig):
            self.assertEqual(sig, signal.SIGTERM)
            events.append(f'stop {pid}')
            living[pid] = False
        def popen(*args, **kwargs):
            events.append('start backend')
            return Mock(pid=13, poll=Mock(return_value=None))
        def launch(*args, **kwargs):
            self.assertIn('LORCA_DATA_DIR=/private/tmp/restart-test/data', args[0])
            events.append('open UI')
        with patch.object(restart, 'cooperative_ui', return_value=True), \
             patch.object(restart, 'ui_processes', side_effect=[{11: ui}, {14: ui}]), \
             patch.object(restart, 'backend_pid', side_effect=[12, 12, None, 13]), \
             patch.object(restart, 'alive', side_effect=lambda pid: living[pid]), \
             patch.object(restart.os, 'kill', side_effect=kill), \
             patch.object(restart.subprocess, 'Popen', side_effect=popen), \
             patch.object(restart.subprocess, 'run', side_effect=launch), \
             patch.object(restart, 'ready', return_value=12345), \
             patch.object(restart, 'mapped_to', return_value=True) as mapped, \
             patch.object(restart, 'output', return_value='14\n'):
            restart.restart(app, Path('/private/tmp/restart-test/data'))
        self.assertEqual(events, ['stop 11', 'stop 12', 'start backend', 'open UI'])
        self.assertEqual(mapped.call_args_list, [call(13, app / 'Contents/MacOS/lorca-core'), call(14, ui)])

    def test_ui_shutdown_timeout_preserves_backend_and_never_force_kills(self):
        with patch.object(restart, 'cooperative_ui', return_value=True), \
             patch.object(restart, 'ui_processes', return_value={11: Path('/tmp/lorca')}), \
             patch.object(restart, 'backend_pid', return_value=12), \
             patch.object(restart.os, 'kill') as kill, \
             patch.object(restart, 'wait_for', side_effect=RuntimeError('timeout')), \
             patch.object(restart.subprocess, 'Popen') as spawn:
            with self.assertRaises(RuntimeError):
                restart.restart(Path('/tmp/lorca.app'), Path('/tmp/data'))
        kill.assert_called_once_with(11, signal.SIGTERM)
        spawn.assert_not_called()

    def test_legacy_ui_is_never_signaled(self):
        with patch.object(restart, 'ui_processes', return_value={11: Path('/tmp/lorca')}), \
             patch.object(restart, 'backend_pid', return_value=12), \
             patch.object(restart, 'cooperative_ui', return_value=False), \
             patch.object(restart.os, 'kill') as kill:
            with self.assertRaisesRegex(RuntimeError, 'Quit it normally'):
                restart.restart(Path('/tmp/lorca.app'), Path('/tmp/data'))
        kill.assert_not_called()

    def test_cooperative_marker_must_match_running_process_inode_and_path(self):
        import json
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'runtime').mkdir()
            path = root / 'lorca'
            marker = root / 'runtime/ui-11.json'
            marker.write_text(json.dumps({'pid': 11, 'protocol': 1, 'executable': str(path), 'inode': 42}))
            with patch.object(restart, 'output', return_value='p11\nftxt\ni42\n'):
                self.assertTrue(restart.cooperative_ui(11, path, root))
                self.assertFalse(restart.cooperative_ui(11, root / 'other', root))
            with patch.object(restart, 'output', return_value='p11\nftxt\ni43\n'):
                self.assertFalse(restart.cooperative_ui(11, path, root))
            marker.write_text('invalid')
            self.assertFalse(restart.cooperative_ui(11, path, root))


if __name__ == '__main__':
    unittest.main()
