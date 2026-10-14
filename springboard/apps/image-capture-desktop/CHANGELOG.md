# Changelog

## 0.1.1

- Moved disk and scanner requests to a serial utility dispatch queue, with replies waking the app on the main queue; removed in-flight polling, enabled sudden termination, and removed the fixed initial window size.

## 0.1.0

- New: Image Capture. The sidebar lists the cameras and SD cards with a DCIM folder and the scanners `scanimage -L` reports. A camera shows its pictures; pick some (or all) and Import copies them into Pictures, keeping their names. A scanner shows its resolution, colour and format; Preview scans a quick look, and Scan saves the page into Downloads (the scan settings name the folder). The Print & Scan pane lists the scanners too.
