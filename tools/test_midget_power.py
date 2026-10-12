"""Pure-function checks for the lab CLI's AMT power control, kernel bench parsing, BIOS search and address picker.
Nothing here contacts a machine (MIDGET_HOST is preset so the import does not probe the network)."""
import importlib.util
import os
import unittest
from pathlib import Path
import xml.etree.ElementTree as ET

os.environ.setdefault("MIDGET_HOST", "10.99.0.200")
spec = importlib.util.spec_from_file_location('midget', Path(__file__).with_name('midget.py'))
midget = importlib.util.module_from_spec(spec)
spec.loader.exec_module(midget)

REPLY = ('<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body>'
         '<g:CIM_AssociatedPowerManagementService xmlns:g="http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_AssociatedPowerManagementService">'
         '<g:PowerState>2</g:PowerState></g:CIM_AssociatedPowerManagementService></s:Body></s:Envelope>')
CHANGE = ('<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body><g:RequestPowerStateChange_OUTPUT xmlns:g="x">'
          '<g:ReturnValue>0</g:ReturnValue></g:RequestPowerStateChange_OUTPUT></s:Body></s:Envelope>')
FAULT = ('<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body><s:Fault><s:Reason>'
         '<s:Text>Access denied</s:Text></s:Reason></s:Fault></s:Body></s:Envelope>')


class PowerXml(unittest.TestCase):
    def test_request_is_well_formed_for_every_action(self):
        for name, code in {"on": 2, "off": 8, "cycle": 5, "reset": 10}.items():
            root = ET.fromstring(midget.power_request_xml("10.99.0.201", name))
            texts = {el.tag.rsplit("}", 1)[-1]: (el.text or "") for el in root.iter()}
            self.assertEqual(texts["PowerState"], str(code))
            self.assertIn("RequestPowerStateChange", texts["Action"])
            self.assertEqual(texts["To"], "http://10.99.0.201:16992/wsman")

    def test_state_request_is_a_get(self):
        root = ET.fromstring(midget.power_state_xml("10.99.0.201"))
        action = next(el.text for el in root.iter() if el.tag.endswith("}Action"))
        self.assertTrue(action.endswith("/transfer/Get"))

    def test_parse_state(self):
        p = midget.parse_power_response(REPLY)
        self.assertEqual(p["power_state"], 2)
        self.assertEqual(midget.power_describe(p), "power state: on (2)")

    def test_parse_change(self):
        p = midget.parse_power_response(CHANGE)
        self.assertEqual(p["return_value"], 0)
        self.assertEqual(midget.power_describe(p, "cycle"), "power cycle: accepted")

    def test_parse_fault_and_garbage(self):
        self.assertEqual(midget.parse_power_response(FAULT)["fault"], "Access denied")
        self.assertIn("not XML", midget.parse_power_response("<html")["fault"])

    def test_redact(self):
        self.assertEqual(midget.redact("pw=Secret1! ok", "Secret1!"), "pw=<amt-password> ok")
        self.assertEqual(midget.redact("text", None), "text")


class KernelBench(unittest.TestCase):
    CYCLIC = "# Min Latencies: 00002 00003\n# Avg Latencies: 00005 00006\n# Max Latencies: 00206 00031\n"

    def test_parse_cyclic(self):
        self.assertEqual(midget.parse_cyclic(self.CYCLIC), {"min": [2, 3], "avg": [5, 6], "max": [206, 31]})

    def test_result_record(self):
        files = {"kernel": "Linux midget 7.2.9-arch1-1 #1 SMP\n", "cyclic-idle": self.CYCLIC,
                 "cyclic-stress": "# Max Latencies: 00337\n", "aplay-32": "underrun!!!\nunderrun!!!\n", "aplay-64": "", "aplay-128": ""}
        rec = midget.kbench_result("stock", files, 60)
        self.assertEqual((rec["kernel"], rec["max_us_idle"], rec["max_us_stress"]), ("7.2.9-arch1-1", 206, 337))
        self.assertEqual(rec["underruns"], {"32": 2, "64": 0, "128": 0})
        self.assertEqual(rec["bench"], "kernel-latency")

    def test_underruns_scale_to_a_minute(self):
        rec = midget.kbench_result("x", {"aplay-32": "underrun!!!\n" * 3}, 30)
        self.assertEqual(rec["underruns"]["32"], 6)


class BiosFind(unittest.TestCase):
    ROWS = ["Intel(R) Manageability Control=Enabled|Disable,Enable", "After Power Loss=Power On|Power Off,Power On", "Wake on LAN=Primary|Disable,Primary"]

    def test_case_insensitive_substring_on_names_with_spaces(self):
        self.assertEqual(midget.bios_find(self.ROWS, "manageability"), [self.ROWS[0]])
        self.assertEqual(midget.bios_find(self.ROWS, "POWER"), self.ROWS[1:2])
        self.assertEqual(midget.bios_find(self.ROWS, ""), self.ROWS)


class HostPicker(unittest.TestCase):
    def test_vlan_first_when_it_answers(self):
        self.assertEqual(midget.pick_host({"192.168.20.140", "10.99.0.200"}), "192.168.20.140")

    def test_lab_nic_when_vlan_is_dead(self):
        self.assertEqual(midget.pick_host({"10.99.0.200"}), "10.99.0.200")

    def test_nothing_answers_falls_back_to_the_lab_nic(self):
        self.assertEqual(midget.pick_host(set()), "10.99.0.200")

    def test_wifi_only(self):
        self.assertEqual(midget.pick_host({"192.168.1.187"}), "192.168.1.187")


if __name__ == "__main__":
    unittest.main()
