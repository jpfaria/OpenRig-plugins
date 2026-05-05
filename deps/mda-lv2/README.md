MDA.LV2
=======

This is a port of the MDA VST plugins to LV2.

MDA plugins are originally by Paul Kellett and were released under the GPL
v2 or later, or the MIT license.  Thanks, Paul!

This port is by David Robillard, and released under the same license.  It is
based on revision 10 of the MDA SVN repository at
<https://mda-vst.svn.sourceforge.net/svnroot/mda-vst>.  It is similar to, but
not 100% compatible with, the original plugins, since some VSTisms have been
changed to be more appropriate for LV2 (e.g. toggle ports).

Approach
--------

To port these plugins to LV2, I wrote the missing code blindly in order to make
things compile and work as an LV2 plugins.  As a result the internal
implementation of the plugins is largely unchanged.  This porting layer, "LVZ",
is not designed to be general, but may be a useful starting point for porting
other VST plugins to LV2.
