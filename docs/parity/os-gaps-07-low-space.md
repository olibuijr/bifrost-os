# Low-space classification

`core::low_space` classifies a volume's available bytes as OK, low or critical using both an absolute threshold and a
fraction of capacity, and provides the notification name and sentence. Headless tests cover boundary behavior and
messages. It has no I/O: the shell still needs to sample volume capacity on a timer and post notifications before this
can be called integrated or verified on Midget.
