#!/usr/bin/env python3
#
# Copyright (C) 2023 The Android Open Source Project
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#      http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
"""Tests for the adb program itself."""

import contextlib
import os
import select
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import unittest

ADB_PATH = os.environ.get('ADB_PATH', 'binaries/linux/adb')

def find_open_port():
    # Find an open port.
    with socket.socket() as s:
        s.bind(("localhost", 0))
        return s.getsockname()[1]

@contextlib.contextmanager
def adb_server():
    """Context manager for an ADB server."""
    port = find_open_port()
    read_pipe, write_pipe = os.pipe()

    if sys.platform == "win32":
        import msvcrt
        write_handle = msvcrt.get_osfhandle(write_pipe)
        os.set_handle_inheritable(write_handle, True)
        reply_fd = str(write_handle)
    else:
        os.set_inheritable(write_pipe, True)
        reply_fd = str(write_pipe)

    proc = subprocess.Popen([ADB_PATH, "-L", "tcp:localhost:{}".format(port),
                             "fork-server", "server",
                             "--reply-fd", reply_fd], close_fds=False)
    try:
        os.close(write_pipe)
        greeting = os.read(read_pipe, 1024)
        assert greeting == b"OK\n", repr(greeting)
        yield port
    finally:
        proc.terminate()
        proc.wait()
        os.close(read_pipe)

@contextlib.contextmanager
def recording_fake_adbd(protocol=socket.AF_INET):
    """Creates a fake ADB daemon that records commands."""
    commands = []
    sync_commands = []
    serversock = socket.socket(protocol, socket.SOCK_STREAM)
    serversock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    serversock.bind(("127.0.0.1", 0))
    serversock.listen(5)
    port = serversock.getsockname()[1]

    # A pipe that is used to signal the thread that it should terminate.
    readsock, writesock = socket.socketpair()

    def _adb_packet(command: bytes, arg0: int, arg1: int, data: bytes) -> bytes:
        bin_command = struct.unpack("<I", command)[0]
        buf = struct.pack("<IIIIII", bin_command, arg0, arg1, len(data), 0,
                          bin_command ^ 0xffffffff)
        buf += data
        return buf

    def _handle_sync(sock):
        """Handles the sync protocol."""
        while True:
            try:
                header = sock.recv(8) # SyncRequest
                if not header:
                    break

                cmd, path_len = struct.unpack("<II", header)
                path = sock.recv(path_len)
                sync_commands.append((cmd, path.decode('utf-8')))

                if cmd == struct.unpack("<I", b"SEND")[0]:
                    # Handle SEND: receive data and DONE.
                    while True:
                        data_header = sock.recv(8)
                        if not data_header:
                            break
                        data_cmd, data_len = struct.unpack("<II", data_header)
                        if data_cmd == struct.unpack("<I", b"DONE")[0]:
                            sock.sendall(struct.pack("<II", struct.unpack("<I", b"OKAY")[0], 0))
                            break
                        file_data = sock.recv(data_len)

                elif cmd == struct.unpack("<I", b"RECV")[0]:
                    # Handle RECV: send some data and DONE.
                    data = b"hello from fake adbd"
                    sock.sendall(struct.pack("<II", struct.unpack("<I", b"DATA")[0], len(data)) + data)
                    sock.sendall(struct.pack("<II", struct.unpack("<I", b"DONE")[0], 0))

                elif cmd == struct.unpack("<I", b"QUIT")[0]:
                    break

            except (ValueError, OSError, ConnectionResetError):
                break

    def _handle():
        rlist = [readsock, serversock]
        cnxn_sent = {}
        while True:
            try:
                read_ready, _, _ = select.select(rlist, [], [])
            except (ValueError, OSError): # Can happen if a socket is closed.
                return

            for ready in read_ready:
                if ready == readsock:
                    for f in rlist:
                        if f.fileno() != -1:
                           f.close()
                    return
                elif ready == serversock:
                    conn, _ = ready.accept()
                    rlist.append(conn)
                else: # Client socket
                    if ready not in cnxn_sent:
                        cnxn_sent[ready] = True
                        # Send CNXN packet
                        ready.sendall(_adb_packet(b"CNXN", 0x01000001, 4096, b"device::"))
                        continue

                    try:
                        header = ready.recv(24) # sizeof(amessage)
                    except ConnectionResetError:
                        header = b'' # Treat as orderly shutdown.

                    if not header:
                        ready.close()
                        rlist.remove(ready)
                        continue

                    command, arg0, arg1, dlen, _, _ = struct.unpack("<IIIIII", header)

                    data = b""
                    if dlen > 0:
                        data = ready.recv(dlen)

                    if command == struct.unpack("<I", b"OPEN")[0]:
                        decoded_data = data.strip(b'\0').decode('utf-8')
                        commands.append(decoded_data)
                        # Reply with OKAY to the OPEN.
                        ready.sendall(_adb_packet(b"OKAY", arg1, arg0, b""))
                        if decoded_data == "sync:":
                            _handle_sync(ready)


    server_thread = threading.Thread(target=_handle)
    server_thread.start()

    try:
        yield port, commands, sync_commands
    finally:
        writesock.send(b'x')
        writesock.close()
        readsock.close()
        server_thread.join()
        serversock.close()

class SmartProtocolTest(unittest.TestCase):
    """Tests for the ADB smart protocol."""

    def test_simple_commands(self):
        """Tests that simple commands are sent to the transport correctly."""
        commands_to_test = [
            (['reboot'], 'reboot:'),
            (['reboot', 'bootloader'], 'reboot:bootloader'),
            (['shell', 'ls'], 'shell:ls'),
            (['exec-out', 'ls'], 'exec:ls'),
            (['root'], 'root:'),
            (['unroot'], 'unroot:'),
            (['remount'], 'remount:'),
            (['tcpip', '5555'], 'tcpip:5555'),
            (['usb'], 'usb:'),
            (['disable-verity'], 'disable-verity:'),
            (['enable-verity'], 'enable-verity:'),
        ]

        with adb_server() as server_port:
            with recording_fake_adbd() as (fake_adbd_port, commands, _):
                # Connect the server to the fake device.
                device_name = "127.0.0.1:{}".format(fake_adbd_port)
                subprocess.check_call([ADB_PATH, '-P', str(server_port), 'connect', device_name])
                subprocess.check_call([ADB_PATH, '-P', str(server_port), '-s', device_name, 'wait-for-device'])

                for adb_args, expected_command in commands_to_test:
                    with self.subTest(adb_args=adb_args):
                        # Clear the recorded commands before each run.
                        commands.clear()
                        # Run the command.
                        subprocess.run([ADB_PATH, '-P', str(server_port), '-s', device_name] + adb_args)
                        self.assertEqual(1, len(commands))
                        self.assertEqual(expected_command, commands[0])

    def test_push_pull(self):
        """Tests that push and pull commands use the sync protocol correctly."""
        with adb_server() as server_port:
            with recording_fake_adbd() as (fake_adbd_port, commands, sync_commands):
                # Connect the server to the fake device.
                device_name = "127.0.0.1:{}".format(fake_adbd_port)
                subprocess.check_call([ADB_PATH, '-P', str(server_port), 'connect', device_name])
                subprocess.check_call([ADB_PATH, '-P', str(server_port), '-s', device_name, 'wait-for-device'])

                # Test push.
                with tempfile.NamedTemporaryFile() as tmp:
                    tmp.write(b"hello")
                    tmp.flush()

                    commands.clear()
                    sync_commands.clear()
                    subprocess.run([ADB_PATH, '-P', str(server_port), '-s', device_name, 'push', tmp.name, '/data/local/tmp/test'])
                    self.assertIn('sync:', commands)
                    self.assertIn((struct.unpack("<I", b"SEND")[0], '/data/local/tmp/test,33188'), sync_commands)

                # Test pull.
                with tempfile.TemporaryDirectory() as tmpdir:
                    commands.clear()
                    sync_commands.clear()
                    subprocess.run([ADB_PATH, '-P', str(server_port), '-s', device_name, 'pull', '/data/local/tmp/test', tmpdir])
                    self.assertIn('sync:', commands)
                    self.assertIn((struct.unpack("<I", b"RECV")[0], '/data/local/tmp/test'), sync_commands)

if __name__ == "__main__":
    unittest.main(verbosity=3)
