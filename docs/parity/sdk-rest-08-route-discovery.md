# Route-discovery model

The SDK adds a namespaced `route-discovery` extension point and a receiver feed for validated reports, refreshes, TTL
expiry, found/lost events and route-picker destinations whose active state comes from `RouteAuth`. Headless tests cover
the feed lifecycle, field validation, expiry boundary, sorting and authorization state.

The model does not send DNS-SD or other network traffic, execute an extension, or connect to the shell's route picker.
The Apple DeviceDiscoveryExtension rows are therefore partial, and no row is parity until the runtime path works on
Midget.
